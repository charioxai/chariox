// MP-07 / MP-08 / MP-11: installer security/lifecycle regressions, not acceptance.
import assert from "node:assert/strict"
import { createServer } from "node:net"
import { mkdtemp, mkdir, readFile, readdir, rm, symlink, writeFile, copyFile } from "node:fs/promises"
import { join } from "node:path"
import { tmpdir } from "node:os"
import test from "node:test"
import { createFixture } from "./fixture.mjs"
import { runMachine, validateRequest } from "./remote.mjs"
import { sshArguments, validateDestination } from "./transport.mjs"

const base = process.env.CHARIOX_BYOM_TEST_STATE ?? join(tmpdir(), "chariox-byom-tests")
await mkdir(base, { recursive: true, mode: 0o700 })
async function harness(t, kernelBytes) {
  const dir = await mkdtemp(join(base, "installer-test-")); t.after(() => rm(dir, { recursive: true, force: true }))
  const home = join(dir, "home"), stage = join(dir, "upload"); await mkdir(home); await mkdir(stage)
  const f = await createFixture(dir, kernelBytes)
  for (const [name, p] of [["release.tar.gz", f.archive], ["release-public-pin", f.releasePublicKey], ["builder-public-pin", f.builderPublicKey]]) await copyFile(p, join(stage, name))
  for (const [name, p] of [["extract-release.py", "../../../deploy/managed-kernel/extract-release.py"], ["verify-image-release.mjs", "../../../deploy/managed-kernel/verify-image-release.mjs"], ["path1-service-policy.mjs", "../../../deploy/managed-kernel/path1-service-policy.mjs"]]) await copyFile(new URL(p, import.meta.url), join(stage, name))
  const calls = [], request = { action: "install", installId: "byom-test", port: 55129, releaseDigest: f.releaseDigest }
  const options = { home, stage, serviceManager: async args => { calls.push(args); return "LoadState=not-found\nFragmentPath=\nDropInPaths=\nActiveState=inactive\nUnitFileState=disabled\n" } }
  const root = join(home, ".local/share/chariox/ssh-machines/byom-test")
  return { dir, home, stage, f, calls, request, options, root }
}
test("MP-11 SSH destination never becomes an option or shell command; normal config/agent retained", () => {
  for (const bad of ["-oProxyCommand=evil", "host;id", "host x", "a\n", "x$(id)", "../foo", ""]) assert.throws(() => validateDestination(bad))
  for (const good of ["linux-lan", "alice@host.example", "10.0.0.3", "alice@[::1]"]) assert.equal(validateDestination(good), good)
  assert.deepEqual(sshArguments("linux-lan", "uname -s"), ["-oBatchMode=yes", "-oClearAllForwardings=yes", "-oForwardAgent=no", "--", "linux-lan", "uname -s"])
})
test("MP-07 invalid roots/IDs/digests/default port are refused before target work", () => {
  const r = { action: "install", installId: "byom-test", port: 55129, releaseDigest: `sha256:${"a".repeat(64)}` }
  for (const delta of [{ installId: "../md-staging" }, { installId: "x\nExecStart=evil" }, { port: 43118 }, { port: 0 }, { releaseDigest: "latest" }, { root: "/" }]) assert.throws(() => validateRequest({ ...r, ...delta }))
})
test("MP-07/MP-08 verified install preserves staging and state; repetition, stop/remove only own unit", async t => {
  const h = await harness(t)
  const old = join(h.home, ".chariox/dev/md-staging"); await mkdir(old, { recursive: true }); await writeFile(join(old, "sentinel"), "keep")
  const result = await runMachine(h.request, h.options)
  assert.equal(result.status, "installed"); assert.equal(result.enrolled, false)
  assert.equal((await runMachine(h.request, h.options)).status, "installed")
  const ownState = join(h.home, ".chariox/dev/ssh-machines/byom-test"); await mkdir(ownState, { recursive: true }); await writeFile(join(ownState, "sentinel"), "state retained")
  const unitPath = join(h.home, ".config/systemd/user/chariox-ssh-byom-test.service")
  const unit = await readFile(unitPath, "utf8")
  assert.match(unit, /CHARIOX_KERNEL_PORT=55129/); assert.match(unit, /UnsetEnvironment=CHARIOX_MANAGED_PROVIDER_TOPOLOGY/)
  assert.doesNotMatch(unit, /sudo|bwrap|ProtectHome=|PrivateTmp=/)
  assert.equal((await runMachine({ ...h.request, action: "stop" }, h.options)).status, "stopped")
  assert.equal((await runMachine({ ...h.request, action: "remove" }, h.options)).status, "removed")
  assert.equal(await readFile(join(old, "sentinel"), "utf8"), "keep")
  assert.equal(await readFile(join(ownState, "sentinel"), "utf8"), "state retained")
  assert.ok(h.calls.flat().every(arg => !arg.includes("chariox-md-staging")))
  assert.deepEqual(await readdir(join(h.home, ".local/share/chariox/ssh-machines")), [])
})
test("MP-07 tampered archive or untrusted public pin never publishes a service", async t => {
  for (const kind of ["archive", "pin"]) {
    const h = await harness(t)
    await writeFile(join(h.stage, kind === "archive" ? "release.tar.gz" : "release-public-pin"), "untrusted")
    await assert.rejects(runMachine(h.request, h.options), /command failed/)
    assert.deepEqual(await readdir(join(h.home, ".config/systemd/user")), [])
  }
})
test("MP-11 symlink ancestor and foreign service collision are refused", async t => {
  const h = await harness(t)
  await symlink(h.dir, join(h.home, ".local"))
  await assert.rejects(runMachine(h.request, h.options), /without links/)
  await rm(join(h.home, ".local"))
  await mkdir(join(h.home, ".config/systemd/user"), { recursive: true })
  await writeFile(join(h.home, ".config/systemd/user/chariox-ssh-byom-test.service"), "foreign")
  await assert.rejects(runMachine(h.request, h.options), /another installation/)
})
test("MP-07 occupied port refuses installation and preserves existing listener", async t => {
  const h = await harness(t), server = createServer()
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve)); t.after(() => new Promise(resolve => server.close(resolve)))
  await assert.rejects(runMachine({ ...h.request, port: server.address().port }, h.options), /already occupied/)
  assert.equal(server.listening, true)
})
test("MP-08 start cannot treat installed as enrolled or relay-ready", async t => {
  const h = await harness(t)
  await assert.rejects(runMachine({ ...h.request, action: "start" }, h.options), /no owned install found|enrollment input required/)
  assert.equal(h.calls.length, 0)
})
test("MP-07 repeat install rejects corruption of an installed signed binary", async t => {
  const h = await harness(t); await runMachine(h.request, h.options)
  await writeFile(join(h.root, "current/usr/local/bin/chariox-kernel"), "corrupt")
  await assert.rejects(runMachine(h.request, h.options), /command failed|corrupted/)
})
test("MP-11 remove refuses unexpected material in the dedicated release root", async t => {
  const h = await harness(t); await runMachine(h.request, h.options)
  await writeFile(join(h.root, "operator-owned-sentinel"), "must retain")
  await assert.rejects(runMachine({ ...h.request, action: "remove" }, h.options), /unexpected/)
  assert.equal(await readFile(join(h.root, "operator-owned-sentinel"), "utf8"), "must retain")
})
test("MP-11 state ancestor link cannot point the future service at staging", async t => {
  const h = await harness(t)
  const state = join(h.home, ".chariox/dev")
  await mkdir(join(h.home, ".chariox"))
  await symlink(h.dir, state)
  await assert.rejects(runMachine(h.request, h.options), /without links/)
  assert.ok(!h.calls.some(args => args[0] === "daemon-reload"))
})
test("MP-11 existing unmarked state is never adopted as an install", async t => {
  const h = await harness(t)
  const state = join(h.home, ".chariox/dev/ssh-machines/byom-test")
  await mkdir(state, { recursive: true }); await writeFile(join(state, "operator-sentinel"), "keep")
  await assert.rejects(runMachine(h.request, h.options), /state.*another installation|unmarked state/)
  assert.equal(await readFile(join(state, "operator-sentinel"), "utf8"), "keep")
})

test("MP-08/MP-11 start enrolls before service start and projects only bound readiness", async t => {
  const h = await harness(t)
  await runMachine(h.request, h.options)
  const order = []
  const safe = { kernelId: "new-kernel", machineId: "new-machine", userId: "owner", publicKeyThumbprint: "bound-key", connected: true }
  const enrollment = { ticket: "fixture-one-use-secret", apiUrl: "http://127.0.0.1:1", userId: "owner" }
  const result = await runMachine({ ...h.request, action: "start" }, { ...h.options, enrollment,
    kernelCommand: async (args, input) => { order.push(args[0]); if (args[0] === "--owner-managed-enroll-stdin") { assert.deepEqual(input, enrollment); return safe } return safe },
    serviceManager: async (args, capture) => { order.push(args[0]); return h.options.serviceManager(args, capture) },
  })
  assert.ok(order.indexOf("--owner-managed-enroll-stdin") < order.indexOf("enable"))
  assert.equal(result.status, "ready")
  assert.equal(result.kernelId, "new-kernel")
  assert.equal(JSON.stringify(result).includes("fixture-one-use-secret"), false)
})

test("MP-08/MP-11 failed redemption never starts service; relay failure stops only owned service", async t => {
  const h = await harness(t)
  await runMachine(h.request, h.options)
  const request = { ...h.request, action: "start" }
  const enrollment = { ticket: "fixture-one-use-secret", apiUrl: "http://127.0.0.1:1", userId: "owner" }
  await assert.rejects(runMachine(request, { ...h.options, enrollment, kernelCommand: async () => { throw new Error("redemption refused") } }), /redemption refused/)
  assert.equal(h.calls.some(args => args[0] === "enable"), false)
  await assert.rejects(runMachine(request, { ...h.options, enrollment, kernelCommand: async args => ({ kernelId: "kernel", machineId: "machine", userId: "owner", publicKeyThumbprint: "key", connected: args[0] === "--owner-managed-enroll-stdin" }) }), /relay/)
  assert.ok(h.calls.some(args => args[0] === "disable" && args.includes("chariox-ssh-byom-test.service")))
})

test("MP-07/MP-11 an occupied companion MCP port fails before install publication", async t => {
  const h = await harness(t)
  const server = createServer(); await new Promise(r => server.listen(0,"127.0.0.1",r))
  t.after(() => new Promise(r => server.close(r)))
  const request = { ...h.request, port: server.address().port - 1 }
  await assert.rejects(runMachine(request, h.options), /already occupied/)
  assert.deepEqual(await readdir(join(h.home,".config/systemd/user")),[])
})

test("MP-07/MP-08 a signed pre-BYOM kernel release cannot publish an install", async t => {
  const h = await harness(t, Buffer.from('#!/bin/sh\nprintf "439\\n"\n'))
  await assert.rejects(runMachine(h.request,h.options), /bootstrap protocol 444/)
  assert.deepEqual(await readdir(join(h.home,".config/systemd/user")),[])
})

test("MP-08/MP-11 inspection recognizes published signed installs and refuses changed service ownership", async t => {
  const h = await harness(t); await runMachine(h.request, h.options)
  assert.equal((await runMachine({ ...h.request, action:"inspect" }, h.options)).status,"installed")
  await writeFile(join(h.home,".config/systemd/user/chariox-ssh-byom-test.service"),"foreign edited service")
  await assert.rejects(runMachine({ ...h.request, action:"inspect" }, h.options), /changed/)
})

// MP-07 / MP-08 / MP-11: review round 00:54, fail-first service ownership/recovery cases.
for (const prior of [{ active: true, enabled: true }, { active: true, enabled: false }, { active: false, enabled: true }, { active: false, enabled: false }]) {
  test(`MP-08/MP-11 repeat readiness failure restores active=${prior.active} enabled=${prior.enabled}`, async t => {
    const h = await harness(t); await runMachine(h.request, h.options)
    let active = prior.active, enabled = prior.enabled
    const calls = []
    const serviceManager = async args => {
      calls.push(args)
      if (args[0] === "show") return `LoadState=loaded\nFragmentPath=${join(h.home, ".config/systemd/user/chariox-ssh-byom-test.service")}\nDropInPaths=\nActiveState=${active ? "active" : "inactive"}\nUnitFileState=${enabled ? "enabled" : "disabled"}\n`
      if (args[0] === "enable") { enabled = true; if (args.includes("--now")) active = true }
      if (args[0] === "disable") { enabled = false; if (args.includes("--now")) active = false }
      if (args[0] === "start") active = true
      if (args[0] === "stop") active = false
    }
    const enrollment = { ticket: "synthetic-ticket", apiUrl: "http://127.0.0.1:1", userId: "owner" }
    await assert.rejects(runMachine({ ...h.request, action: "start" }, { ...h.options, enrollment, serviceManager,
      kernelCommand: async args => ({ kernelId: "kernel", machineId: "machine", userId: "owner", publicKeyThumbprint: "key", connected: args[0] !== "--owner-managed-ready" }),
    }), /relay-ready/)
    assert.equal(active, prior.active, "pre-existing running work must survive readiness failure")
    assert.equal(enabled, prior.enabled, "existing boot enablement must survive readiness failure")
    if (prior.active && prior.enabled) assert.ok(calls.every(args => args[0] === "show"), "repeat must not mutate existing service")
  })
}

test("MP-07/MP-11 removal retries after unit unlink and reload failure, retaining private state", async t => {
  const h = await harness(t); await runMachine(h.request, h.options)
  const unit = join(h.home, ".config/systemd/user/chariox-ssh-byom-test.service")
  const state = join(h.home, ".chariox/dev/ssh-machines/byom-test/sentinel")
  await writeFile(state, "keep private state")
  await assert.rejects(runMachine({ ...h.request, action: "remove" }, { ...h.options, serviceManager: async args => {
    if (args[0] === "daemon-reload") throw new Error("transient reload failure")
    return h.options.serviceManager(args)
  } }), /transient reload failure/)
  await assert.rejects(readFile(unit), { code: "ENOENT" })
  assert.equal((await runMachine({ ...h.request, action: "inspect" }, h.options)).status, "installed", "inspection must retain published ownership through removal")
  await assert.rejects(runMachine(h.request, h.options), /removal.*remove/, "pending removal must not silently revive a kernel")
  assert.equal((await runMachine({ ...h.request, action: "remove" }, h.options)).status, "removed")
  assert.equal(await readFile(state, "utf8"), "keep private state")
  assert.equal((await runMachine({ ...h.request, action: "inspect" }, h.options)).status, "absent")
  assert.equal((await runMachine(h.request, h.options)).status, "installed", "same ID can be installed after completed removal")
})

test("MP-11 interrupted removal still rejects replacement foreign units and active jobs", async t => {
  const h = await harness(t); await runMachine(h.request, h.options)
  const request = { ...h.request, action: "remove" }
  await assert.rejects(runMachine(request, { ...h.options, serviceManager: async args => {
    if (args[0] === "daemon-reload") throw new Error("interrupted removal")
    return h.options.serviceManager(args)
  } }), /interrupted removal/)
  const unit = join(h.home, ".config/systemd/user/chariox-ssh-byom-test.service")
  await writeFile(unit, "foreign replacement")
  await assert.rejects(runMachine(request, h.options), /changed/)
  await rm(unit)
  await assert.rejects(runMachine(request, { ...h.options, serviceManager: async args => {
    if (args[0] === "show") return "LoadState=loaded\nFragmentPath=\nDropInPaths=\nActiveState=active\nUnitFileState=disabled\n"
    return h.options.serviceManager(args)
  } }), /stopped/)
  assert.equal((await readFile(join(h.root, "install.json"))).length > 0, true)
})

test("MP-11 a missing unit without proven removal intent cannot be adopted", async t => {
  const h = await harness(t); await runMachine(h.request, h.options)
  await rm(join(h.home, ".config/systemd/user/chariox-ssh-byom-test.service"))
  await assert.rejects(runMachine({ ...h.request, action: "remove" }, h.options), { code: "ENOENT" })
  await assert.rejects(runMachine({ ...h.request, action: "inspect" }, h.options), { code: "ENOENT" })
  assert.ok((await readFile(join(h.root, "install.json"))).length)
})
