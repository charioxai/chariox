// MP-07 / MP-08 / MP-11: a live isolated Setup kernel suppresses redundant installation offers.
import assert from "node:assert/strict"
import { createServer } from "node:net"
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import type { RelayCloudProfile } from "./preferences.js"
import { hasLocalKernel, startLocalKernelSetup } from "./local-kernel-setup.js"

test("MP-08 setup offer observes fresh per-install presence and ignores stale records", async t => {
  const home = mkdtempSync(join(process.env.CHARIOX_BYOM_TEST_STATE ?? tmpdir(), "setup-offer-"))
  const keys = ["HOME", "XDG_CONFIG_HOME", "CHARIOX_HOME", "CHARIOX_ACTIVE_KERNEL_REGISTRY_DIR", "CHARIOX_KERNEL_PORT"]
  const previous = new Map(keys.map(key => [key, process.env[key]]))
  t.after(() => {
    for (const [key, value] of previous) { if (value === undefined) delete process.env[key]; else process.env[key] = value }
    rmSync(home, { recursive: true, force: true })
  })
  for (const key of keys) delete process.env[key]
  process.env.HOME = home
  const server = createServer()
  await new Promise<void>(r => server.listen(0, "127.0.0.1", r))
  const address = server.address(); assert.ok(address && typeof address !== "string")
  process.env.CHARIOX_KERNEL_PORT = String(address.port)
  await new Promise<void>(r => server.close(() => r()))
  const registry = join(home, ".chariox/dev/ssh-machines/local/kernels/active")
  mkdirSync(registry, { recursive: true })
  const file = join(registry, "setup-kernel.json")
  const record = { schema_version: 1, kernel_id: "setup-kernel", machine_id: "setup-machine", host: "127.0.0.1", port: 55139, heartbeat_at_ms: Date.now() }
  writeFileSync(file, JSON.stringify(record))
  assert.equal(await hasLocalKernel(), true, "running isolated kernel must suppress setup offer")
  writeFileSync(file, JSON.stringify({ ...record, heartbeat_at_ms: Date.now() - 60_000 }))
  assert.equal(await hasLocalKernel(), false, "stale presence must not suppress repair/setup")
  process.env.CHARIOX_HOME = join(home, ".chariox/dev/ssh-machines/local")
  writeFileSync(file, JSON.stringify(record))
  assert.equal(await hasLocalKernel(), true, "explicit kernel home uses the same freshness rule")
})

// MP-07 / MP-08 / MP-11: run the actual login-offer handoff from a marked CLI release.
for (const installId of ["local", "second"]) {
  test(`MP-08 login Setup handoff retains ${installId} install on nondefault port`, async t => {
    const home = mkdtempSync(join(process.env.CHARIOX_BYOM_TEST_STATE ?? tmpdir(), "setup-handoff-"))
    const priorHome = process.env.HOME, priorExec = process.execPath, priorKernelHome = process.env.CHARIOX_HOME
    t.after(() => {
      process.execPath = priorExec
      if (priorHome === undefined) delete process.env.HOME; else process.env.HOME = priorHome
      if (priorKernelHome === undefined) delete process.env.CHARIOX_HOME; else process.env.CHARIOX_HOME = priorKernelHome
      rmSync(home, { recursive: true, force: true })
    })
    process.env.HOME = home
    process.env.CHARIOX_HOME = join(home, ".chariox/dev/foreign-staging")
    const root = join(home, ".local/share/chariox/ssh-machines", installId)
    const bin = join(root, "releases", "a".repeat(64), "bin")
    mkdirSync(bin, { recursive: true })
    symlinkSync(`releases/${"a".repeat(64)}`, join(root, "current"))
    const marker = { format: "chariox.ssh-machine-install.v1", installId, port: 55149, service: `chariox-ssh-${installId}.service`, releaseDigest: `sha256:${"a".repeat(64)}` }
    writeFileSync(join(root, "install.json"), JSON.stringify(marker), { mode: 0o600 })
    if (installId === "second") {
      const local = join(home, ".local/share/chariox/ssh-machines/local")
      mkdirSync(local, { recursive: true })
      writeFileSync(join(local, "install.json"), JSON.stringify({ ...marker, installId: "local", service: "chariox-ssh-local.service", port: 55139 }), { mode: 0o600 })
    }
    writeFileSync(join(bin, "chariox"), "fixture CLI")
    writeFileSync(join(bin, "chariox-setup"), `#!/usr/bin/env node\nconst fs = require("node:fs"); fs.writeFileSync(${JSON.stringify(join(home, "handoff.json"))}, JSON.stringify({ args: process.argv.slice(2), inheritedKernelHome: process.env.CHARIOX_HOME ?? null }))\n`, { mode: 0o700 })
    process.execPath = join(root, "current/bin/chariox")
    const profile = { apiUrl: "https://chariox.test", userId: "owner" } as RelayCloudProfile
    await startLocalKernelSetup(profile)
    const handoff = JSON.parse(readFileSync(join(home, "handoff.json"), "utf8"))
    assert.deepEqual(handoff.args, ["--api-url", profile.apiUrl, "--user-id", "owner", "--id", installId, "--port", "55149", "--repair"])
    assert.equal(handoff.inheritedKernelHome, null, "handoff must still isolate inherited kernel state")
    const explanation = "MP-07/MP-08/MP-11: Setup failed: another install operation owns this install ID; confirm no installer is running, then remove only the empty lock directory."
    writeFileSync(join(bin, "chariox-setup"), `#!/usr/bin/env node\nprocess.stderr.write(${JSON.stringify(explanation + "\n")});process.exitCode=1\n`, { mode: 0o700 })
    const notices: string[] = []
    await assert.rejects(startLocalKernelSetup(profile, message => notices.push(message)), error => {
      assert.ok(error instanceof Error && error.message.includes(explanation), "the final TUI failure must retain the recovery explanation")
      return true
    })
    assert.ok(notices.join("").includes(explanation), "the TUI must receive Setup's recovery explanation")
    writeFileSync(join(bin, "chariox-setup"), '#!/usr/bin/env node\nprocess.stderr.write(Buffer.alloc(32769,120));setInterval(()=>{},1000)\n', { mode: 0o700 })
    const bounded: string[] = []
    await assert.rejects(startLocalKernelSetup(profile, message => bounded.push(message)), /Setup failed/)
    assert.ok(Buffer.byteLength(bounded.join("")) <= 32768, "stdout and stderr share the output bound")
    for (const invalid of [{ installId: "foreign" }, { port: 43118 }, { port: "55149" }, { port: 65535 }, { service: "chariox-md-staging.service" }]) {
      writeFileSync(join(root, "install.json"), JSON.stringify({ ...marker, ...invalid }), { mode: 0o600 })
      await assert.rejects(startLocalKernelSetup(profile), /invalid identity or port/, "invalid marked selection must not fall back to another install")
    }
    rmSync(join(root, "install.json"))
    await assert.rejects(startLocalKernelSetup(profile), /marker is missing/)
    symlinkSync(join(bin, "chariox"), join(root, "install.json"))
    await assert.rejects(startLocalKernelSetup(profile), /regular file/)
  })
}
