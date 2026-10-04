import assert from "node:assert/strict"
import test from "node:test"
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { defaultKernelUnixSocketPath } from "@chariox/kernel-client/kernel-unix-socket-path"
import { kernelUnixClient } from "./kernel-unix-client.js"

test("CLI discovers maximal slice identity from the kernel registry and preserves explicit overrides", async () => {
  const root = mkdtempSync(join(tmpdir(), "chx-discovery-"))
  const names = ["CHARIOX_HOME", "CHARIOX_DAEMON_ID", "CHARIOX_DAEMON_SOCKET", "CHARIOX_KERNEL_HOST", "CHARIOX_KERNEL_PORT"] as const
  const previous = names.map(name => process.env[name])
  const id = `slice:${"a".repeat(64)}:${"b".repeat(64)}`
  const home = join(root, "deep/".repeat(40), "kernel")
  try {
    for (const name of names) delete process.env[name]
    process.env.CHARIOX_HOME = home
    mkdirSync(join(home, "kernels"), { recursive: true })
    writeFileSync(join(home, "kernels/registry.json"), JSON.stringify({ kernels: { "127.0.0.1:43118": { kernel_id: id } } }))
    const client = kernelUnixClient()
    assert.equal(client.socketPath, `ws+unix://${defaultKernelUnixSocketPath(id, home)}`)
    await client.close()
    process.env.CHARIOX_DAEMON_ID = id
    assert.equal(kernelUnixClient().socketPath, client.socketPath)
    process.env.CHARIOX_DAEMON_SOCKET = "/tmp/private/override.sock"
    assert.equal(kernelUnixClient().socketPath, "ws+unix:///tmp/private/override.sock")
    assert.equal(kernelUnixClient("/tmp/private/explicit.sock").socketPath, "ws+unix:///tmp/private/explicit.sock")
  } finally {
    names.forEach((name, i) => { if (previous[i] === undefined) delete process.env[name]; else process.env[name] = previous[i] })
    rmSync(root, { recursive: true, force: true })
  }
})
