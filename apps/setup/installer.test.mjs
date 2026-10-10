// MP-07 / MP-08 / MP-11: mocked release server + actual shared target install filesystem.
import assert from "node:assert/strict"
import { createServer } from "node:http"
import { generateKeyPairSync } from "node:crypto"
import { copyFile, cp, lstat, mkdir, mkdtemp, readFile, readlink, realpath, readdir, rm, symlink, writeFile } from "node:fs/promises"
import { join } from "node:path"
import { tmpdir } from "node:os"
import test from "node:test"
import { installLocal, targetPlatform, publicUrl } from "./installer.mjs"
import { setupFixture } from "./fixture.mjs"
import { launchdDefinition, launchdManager } from "./launchd.mjs"
import { runMachine } from "../kernel/ssh-machine/remote.mjs"
const extractorSource = await readFile(new URL("../../deploy/managed-kernel/extract-release.py", import.meta.url), "utf8")
const state = await realpath(process.env.CHARIOX_BYOM_TEST_STATE ?? tmpdir())
async function harness(t, overrides = {}) {
  const dir = await mkdtemp(join(state, "setup-test-")); t.after(() => rm(dir, { recursive: true, force: true }))
  const home = join(dir, "home"); await mkdir(home)
  const f = await setupFixture(dir, overrides), responses = new Map(), requests = [], calls = []
  const serve = async fixture => {
    const path = `/v${fixture.version}/chariox-${fixture.version}-${fixture.platform}`
    responses.set(`${path}.manifest.json`, fixture.manifest); responses.set(`${path}.manifest.sig`, fixture.signature); responses.set(`${path}.tar.gz`, await readFile(fixture.archive))
  }
  await serve(f)
  const server = createServer((req, res) => { requests.push(req.url); if (!responses.has(req.url)) { res.writeHead(404); return res.end() }; res.end(responses.get(req.url)) })
  await new Promise(r => server.listen(0, "127.0.0.1", r)); t.after(() => new Promise(r => server.close(r)))
  const root = join(home, ".local/share/chariox/ssh-machines/local")
  const options = { home, platform: f.platform, version: f.version, publicKeyHex: f.publicKeyHex, releaseBase: `http://127.0.0.1:${server.address().port}`, apiUrl: "http://127.0.0.1:1", extractorSource, installOnly: true, serviceManager: async args => { calls.push(args); return "LoadState=not-found\nFragmentPath=\nDropInPaths=\nActiveState=inactive\nUnitFileState=disabled\n" } }
  return { dir, home, f, responses, requests, calls, root, options, serve }
}
test("MP-07 platform and URL admission reject credential URLs and unsupported targets", () => {
  assert.equal(targetPlatform("linux", "x64"), "linux-x64"); assert.equal(targetPlatform("darwin", "arm64"), "darwin-arm64")
  for (const pair of [["win32", "x64"], ["linux", "arm64"]]) assert.throws(() => targetPlatform(...pair))
  for (const value of ["http://remote.example", "https://user:password@remote.example", "https://remote.example?code=x"]) assert.throws(() => publicUrl(value))
})
test("MP-07 signed CLI/kernel install is idempotent and retains staging plus runtime state on remove", async t => {
  const h = await harness(t)
  const staging = join(h.home, ".chariox/dev/md-staging"); await mkdir(staging, { recursive: true }); await writeFile(join(staging, "sentinel"), "keep")
  assert.equal((await installLocal(h.options)).status, "installed")
  assert.equal((await installLocal(h.options)).releaseDigest, h.f.digest)
  assert.equal(await readlink(join(h.home, ".local/bin/chariox")), `${h.root}/current/bin/chariox`)
  assert.equal((await installLocal({ ...h.options, action: "remove" })).stateRetained, true)
  assert.equal(await readFile(join(staging, "sentinel"), "utf8"), "keep")
  assert.ok((await readdir(join(h.home, ".chariox/dev/ssh-machines/local"))).includes("ssh-install-owner.json"))
  assert.deepEqual(await readdir(join(h.home, ".local/bin")), [])
})
test("MP-07 signature failure refuses execution before archive download and service publication", async t => {
  const h = await harness(t); h.responses.set(`/v${h.f.version}/chariox-${h.f.version}-linux-x64.manifest.sig`, "a".repeat(128))
  await assert.rejects(installLocal(h.options), /signature/)
  assert.equal(h.requests.some(p => p.endsWith("tar.gz")), false); assert.equal(h.calls.length, 0)
})
test("MP-07 signed metadata cannot authorize a tampered payload or a different platform", async t => {
  const h = await harness(t); h.responses.set(`/v${h.f.version}/chariox-${h.f.version}-linux-x64.tar.gz`, Buffer.from("corrupt"))
  await assert.rejects(installLocal(h.options), /command failed/)
  assert.ok(!h.calls.some(args => ["enable", "daemon-reload"].includes(args[0])))
})
test("MP-11 self-enrollment reads only stdin and shares runtime ready gating, without copying human credentials", async t => {
  const h = await harness(t), seen = [], identity = { kernelId: "kernel", machineId: "machine", userId: "owner", publicKeyThumbprint: "pin", connected: true }
  const options = { ...h.options, installOnly: false, ticket: "synthetic-single-use-code", kernelCommand: async (args, input) => { seen.push([args, input ? { ...input } : null]); return identity } }
  assert.equal((await installLocal(options)).status, "ready")
  assert.deepEqual(seen[0], [["--owner-managed-self-enroll-stdin"], { ticket: "synthetic-single-use-code", apiUrl: "http://127.0.0.1:1" }])
  assert.equal(options.ticket, ""); assert.ok(!h.calls.flat().includes("synthetic-single-use-code"))
})
test("MP-07 explicit upgrade is atomic, verifies original pins and rolls back failed readiness", async t => {
  const h = await harness(t); await installLocal(h.options)
  const nextDir = join(h.dir, "next"); await mkdir(nextDir)
  const next = await setupFixture(nextDir, { version: "0.4.0", keys: h.f.keys }); await h.serve(next)
  const before = await readFile(join(h.root, "install.json")), state = join(h.home, ".chariox/dev/ssh-machines/local/sentinel"); await writeFile(state, "private state stays")
  let readyCalls = 0
  const upgrade = { ...h.options, version: next.version, action: "upgrade", kernelCommand: async () => { if (++readyCalls === 1) return { connected: true }; throw new Error("relay unavailable") } }
  await assert.rejects(installLocal(upgrade), /relay unavailable/)
  assert.deepEqual(await readFile(join(h.root, "install.json")), before)
  assert.equal(await readlink(join(h.root, "current")), `releases/${h.f.digest.slice(7)}`)
  assert.equal((await installLocal({ ...upgrade, kernelCommand: async () => ({ connected: true }) })).status, "upgraded")
  assert.equal(await readlink(join(h.root, "current")), `releases/${next.digest.slice(7)}`)
  assert.equal(await readFile(state, "utf8"), "private state stays")
})
for (const cached of [false, true]) {
  for (const rollback of [false, true]) {
    test(`MP-07/MP-08/MP-11 delayed upgrade retains staging and lock: cached=${cached} rollback=${rollback}`, async t => {
      const h = await harness(t); await installLocal(h.options)
      const nextDir = join(h.dir, "next"); await mkdir(nextDir)
      const next = await setupFixture(nextDir, { version: "0.4.0", keys: h.f.keys }); await h.serve(next)
      if (cached) await cp(next.bundle, join(h.root, "releases", next.digest.slice(7)), { recursive: true })
      const parent = join(h.home, ".local/share/chariox/ssh-machines")
      const lock = join(parent, ".local.lock"), before = await readFile(join(h.root, "install.json"))
      const sentinel = join(h.home, ".chariox/dev/ssh-machines/local/sentinel")
      await writeFile(sentinel, "private state stays")
      let readiness = 0, stops = 0, stages
      async function held(checkImage = false) {
        // Keep readiness/service I/O pending long enough for premature finally cleanup to run.
        await new Promise(resolve => setTimeout(resolve, 100))
        assert.ok((await lstat(lock)).isDirectory(), "upgrade must retain its exclusive lock")
        const names = (await readdir(parent)).filter(name => name.startsWith(".local.stage-"))
        assert.equal(names.length, 1, "upgrade must retain its staging tree until settlement")
        stages ??= names
        assert.deepEqual(names, stages)
        if (checkImage) assert.ok((await lstat(join(parent, names[0], "image"))).isDirectory(), "initial readiness must retain the staged image")
        await assert.rejects(runMachine({ action: "remove", installId: "local", port: 55139, releaseDigest: h.f.digest }, { home: h.home, serviceManager: h.options.serviceManager }), /another install operation owns/)
      }
      const upgrade = installLocal({ ...h.options, version: next.version, action: "upgrade",
        kernelCommand: async () => {
          await held(++readiness === 1)
          if (rollback && readiness === 2) throw new Error("synthetic readiness failure")
          return { connected: true, kernelId: "kernel", machineId: "machine", userId: "owner", publicKeyThumbprint: "pin" }
        },
        serviceManager: async args => {
          if (args[0] === "disable" && ++stops === 2) await held() // Rollback also owns the lock until service restoration completes.
          return h.options.serviceManager(args)
        },
      })
      if (rollback) await assert.rejects(upgrade, /synthetic readiness failure/)
      else assert.equal((await upgrade).status, "upgraded")
      assert.equal(readiness, 2)
      assert.equal(stops, rollback ? 2 : 1)
      await assert.rejects(lstat(lock), { code: "ENOENT" })
      assert.deepEqual((await readdir(parent)).filter(name => name.startsWith(".local.stage-") || name.startsWith(".setup-")), [])
      assert.equal(await readlink(join(h.root, "current")), `releases/${(rollback ? h.f.digest : next.digest).slice(7)}`)
      if (rollback) assert.deepEqual(await readFile(join(h.root, "install.json")), before)
      else assert.equal(JSON.parse(await readFile(join(h.root, "install.json"))).releaseDigest, next.digest)
      assert.equal(await readFile(sentinel, "utf8"), "private state stays")
    })
  }
}
test("MP-07 an upgrade signed by an unapproved replacement key cannot stop the current service", async t => {
  const h = await harness(t); await installLocal(h.options); h.calls.length = 0
  const dir = join(h.dir, "foreign"); await mkdir(dir); const f = await setupFixture(dir, { version: "0.4.0" }); await h.serve(f)
  await assert.rejects(installLocal({ ...h.options, version: f.version, publicKeyHex: f.publicKeyHex, action: "upgrade" }), /signature/)
  assert.ok(!h.calls.some(args => args[0] === "disable"))
})
test("MP-11 foreign CLI and symlink state are refused without service mutation", async t => {
  const h = await harness(t); await mkdir(join(h.home, ".local/bin"), { recursive: true }); await writeFile(join(h.home, ".local/bin/chariox"), "owned by user")
  await assert.rejects(installLocal(h.options), /another install/); assert.equal(h.requests.length, 0); assert.equal(h.calls.length, 0)
})
test("MP-07/MP-08 macOS uses a distinct per-user launchd unit through the same core", async t => {
  const h = await harness(t, { platform: "darwin-arm64" }); await installLocal(h.options)
  const plist = await readFile(join(h.home, "Library/LaunchAgents/com.chariox.kernel.local.plist"), "utf8")
  assert.ok(plist.includes("CHARIOX_KERNEL_PORT</key><string>55139")); assert.ok(plist.includes("-u</string><string>CHARIOX_LOG_DIR"))
  assert.ok(!plist.includes("md-staging"))
  await installLocal({ ...h.options, action: "remove" })
})
test("MP-11 launchd refuses a loaded foreign service; stop touches only owned label", async () => {
  const calls = []
  const foreign = launchdManager("/home/user", "local", async (_program, args) => { calls.push(args); return "path = /foreign.plist\n" })
  await assert.rejects(foreign(["show", "com.chariox.kernel.local.plist"]), /foreign/)
  const own = launchdManager("/home/user", "local", async (_program, args) => { calls.push(args); return "path = /home/user/Library/LaunchAgents/com.chariox.kernel.local.plist\n" })
  await own(["disable", "--now", "com.chariox.kernel.local.plist"])
  assert.ok(calls.some(args => args[0] === "bootout" && args[1].endsWith("/com.chariox.kernel.local")))
})
test("MP-07/MP-08 multiple installs use distinct roots, service labels, ports and CLI aliases", async t => {
  const h = await harness(t); await installLocal(h.options)
  const second = { ...h.options, installId: "second", port: 55149 }
  await installLocal(second)
  assert.equal(await readlink(join(h.home, ".local/bin/chariox-second")), `${h.home}/.local/share/chariox/ssh-machines/second/current/bin/chariox`)
  await installLocal({ ...second, action: "remove" })
  assert.equal(await readlink(join(h.home, ".local/bin/chariox")), `${h.root}/current/bin/chariox`)
})
test("MP-07 verified release downloads support bounded public asset redirects", async t => {
  const h = await harness(t)
  const original = h.responses.get(`/v${h.f.version}/chariox-${h.f.version}-linux-x64.tar.gz`)
  h.responses.set('/redirected-archive', original)
  const server = createServer((req, res) => { res.writeHead(302, { location: `${h.options.releaseBase}/redirected-archive` }); res.end() })
  await new Promise(r => server.listen(0, "127.0.0.1", r)); t.after(() => new Promise(r => server.close(r)))
  // The signature is checked at the release authority; only public artifact bytes redirect.
  const { prepareRelease } = await import("./installer.mjs")
  const stage = join(h.dir, "redirect-stage"); await mkdir(stage)
  const realFetch = globalThis.fetch
  globalThis.fetch = (url, init) => String(url).endsWith('.tar.gz') ? realFetch(`http://127.0.0.1:${server.address().port}`, init) : realFetch(url, init)
  try { assert.equal(await prepareRelease({ ...h.options, platform: "linux-x64", stage }), h.f.digest) }
  finally { globalThis.fetch = realFetch }
})
test("MP-07 interrupted activation rolls back its signed journal before repair and retains identity state", async t => {
  const h = await harness(t); await installLocal(h.options)
  const previous = JSON.parse(await readFile(join(h.root, "install.json")))
  const root = join(h.dir, "interrupted"); await mkdir(root); const next = await setupFixture(root, { version: "0.4.0", keys: h.f.keys }); await h.serve(next)
  // Retain a verified target from a failed upgrade, then reproduce the post-activation/pre-marker seam.
  let attempts = 0
  await assert.rejects(installLocal({ ...h.options, version: next.version, action: "upgrade", kernelCommand: async () => { if (++attempts === 1) return { connected: true }; throw new Error("interrupted readiness") } }))
  await rm(join(h.root, "current")); await symlink(`releases/${next.digest.slice(7)}`, join(h.root, "current"))
  await writeFile(join(h.root, "upgrade.json"), JSON.stringify({ previous, target: next.digest }), { mode: 0o600 })
  assert.equal((await installLocal({ ...h.options, action: "repair" })).releaseDigest, previous.releaseDigest)
  assert.equal(await readlink(join(h.root, "current")), `releases/${previous.releaseDigest.slice(7)}`)
  await assert.rejects(readFile(join(h.root, "upgrade.json")))
})
test("MP-07 invalid release selection is refused before creating install or state paths", async t => {
  const h = await harness(t)
  await assert.rejects(installLocal({ ...h.options, version: "invalid" }), /pinned release/)
  assert.deepEqual(await readdir(h.home), [])
  assert.deepEqual(h.requests, [])
})

test("MP-08/MP-11 launchd preserves independent active/enabled state during rollback operations", async () => {
  const id = "second", home = "/home/user", label = "com.chariox.kernel.second"
  const domain = `gui/${process.getuid()}`, calls = []
  let loaded = true, enabled = true
  const manager = launchdManager(home, id, async (_program, args) => {
    calls.push(args)
    if (args[0] === "print" && args[1] === `${domain}/${label}`) {
      if (!loaded) throw new Error("job absent")
      return `path = ${home}/Library/LaunchAgents/${label}.plist\n`
    }
    if (args[0] === "print-disabled") return `disabled services = {\n "${label}" => ${!enabled}\n}\n`
    if (args[0] === "bootout") loaded = false
    if (args[0] === "bootstrap") loaded = true
    if (args[0] === "disable") enabled = false
    if (args[0] === "enable") enabled = true
    return ""
  })
  const unit = `${label}.plist`, show = ["show", unit, "--property=ActiveState", "--property=UnitFileState"]
  assert.match(await manager(show), /ActiveState=active\nUnitFileState=enabled/)
  await manager(["disable", unit])
  assert.equal(loaded, true, "disable without --now must preserve running job")
  assert.match(await manager(show), /ActiveState=active\nUnitFileState=disabled/)
  await manager(["stop", unit])
  assert.match(await manager(show), /ActiveState=inactive\nUnitFileState=disabled/)
  await manager(["enable", unit])
  assert.equal(loaded, false, "enable without --now must not bootstrap a job")
  await manager(["start", unit])
  assert.match(await manager(show), /ActiveState=active\nUnitFileState=enabled/)
  assert.ok(calls.filter(args => ["bootout", "bootstrap", "disable", "enable"].includes(args[0])).every(args => args.includes(`${domain}/${label}`) || args.includes(`${home}/Library/LaunchAgents/${label}.plist`)))
})

for (const platform of ["linux-x64", "darwin-arm64"]) {
  test(`MP-07/MP-08/MP-11 ${platform} repairs the original service after PATH changes`, async t => {
    const h = await harness(t, { platform })
    await installLocal(h.options)
    const unit = platform === "linux-x64"
      ? join(h.home, ".config/systemd/user/chariox-ssh-local.service")
      : join(h.home, "Library/LaunchAgents/com.chariox.kernel.local.plist")
    const previous = await readFile(unit)
    const oldPath = process.env.PATH
    try {
      process.env.PATH = `${h.home}/.local/bin:${oldPath}`
      await rm(unit)
      assert.equal((await installLocal({ ...h.options, action: "repair" })).status, "installed")
      assert.deepEqual(await readFile(unit), previous)
      await rm(unit)
      const markerPath = join(h.root, "install.json")
      const marker = JSON.parse(await readFile(markerPath))
      marker.unitContent += "tampered policy"
      await writeFile(markerPath, JSON.stringify(marker))
      await assert.rejects(installLocal({ ...h.options, action: "repair" }), /repair cannot change service policy/)
      await assert.rejects(readFile(unit), { code: "ENOENT" })
    } finally { process.env.PATH = oldPath }
  })
}
