// MP-07 / MP-08 / MP-11: a live isolated Setup kernel suppresses redundant installation offers.
import assert from "node:assert/strict"
import { createServer } from "node:net"
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { hasLocalKernel } from "./local-kernel-setup.js"

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
