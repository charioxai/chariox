import assert from "node:assert/strict"
import test from "node:test"
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { defaultKernelUnixSocketPath } from "@chariox/kernel-client/kernel-unix-socket-path"
import { CommandFailure } from "./agent-command-table.js"
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
    process.env.CHARIOX_DAEMON_ID = "different-runtime-alias"
    assert.equal(kernelUnixClient().socketPath, client.socketPath)
    rmSync(join(home, "kernels/registry.json"))
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

// Row 479: an external agent has no CHARIOX_* variables; Setup keeps its kernel under ~/.chariox/dev/machines/<id>.
function withBareHome(run: (home: string) => void): void {
  const home = mkdtempSync(join(tmpdir(), "chx-bare-home-"))
  const names = ["HOME", "CHARIOX_HOME", "CHARIOX_DAEMON_ID", "CHARIOX_DAEMON_SOCKET", "CHARIOX_KERNEL_HOST", "CHARIOX_KERNEL_PORT", "XDG_CONFIG_HOME"] as const
  const previous = names.map(name => process.env[name])
  try {
    for (const name of names) delete process.env[name]
    process.env.HOME = home
    run(home)
  } finally {
    names.forEach((name, i) => { if (previous[i] === undefined) delete process.env[name]; else process.env[name] = previous[i] })
    rmSync(home, { recursive: true, force: true })
  }
}

function startKernelHome(home: string, kernelHome: string, kernelId: string, port: number): void {
  const active = join(kernelHome, "kernels/active")
  mkdirSync(active, { recursive: true })
  writeFileSync(join(active, `${kernelId}.json`), JSON.stringify({
    schema_version: 1, kernel_id: kernelId, machine_id: "machine-a", host: "127.0.0.1", port, heartbeat_at_ms: Date.now(),
  }))
}

test("a bare shell finds the one Setup kernel without CHARIOX_HOME", () => withBareHome(home => {
  const kernelHome = join(home, ".chariox/dev/machines/local")
  startKernelHome(home, kernelHome, "kernel-setup", 55139)
  assert.equal(kernelUnixClient().socketPath, `ws+unix://${defaultKernelUnixSocketPath("kernel-setup", kernelHome)}`)
}))

test("a bare shell finds a kernel running in the default home on another port", () => withBareHome(home => {
  startKernelHome(home, join(home, ".chariox"), "kernel-dev", 43999)
  assert.equal(kernelUnixClient().socketPath, `ws+unix://${defaultKernelUnixSocketPath("kernel-dev", join(home, ".chariox"))}`)
}))

test("with no running kernel the refusal names the step to take", () => withBareHome(() => {
  assert.throws(() => kernelUnixClient(), (error: unknown) => error instanceof CommandFailure
    && error.code === "kernel_unreachable" && /Start it/.test(error.message) && !/ENOENT|registry/.test(error.message))
}))

test("several running kernels are listed, not guessed", () => withBareHome(home => {
  startKernelHome(home, join(home, ".chariox/dev/machines/local"), "kernel-a", 55139)
  startKernelHome(home, join(home, ".chariox/dev/machines/md"), "kernel-b", 55141)
  assert.throws(() => kernelUnixClient(), (error: unknown) => error instanceof CommandFailure
    && error.code === "kernel_ambiguous" && error.message.includes("machines/local") && error.message.includes("machines/md") && /CHARIOX_HOME/.test(error.message))
}))

test("an explicit CHARIOX_HOME is never widened to the Setup kernels", () => withBareHome(home => {
  startKernelHome(home, join(home, ".chariox/dev/machines/local"), "kernel-setup", 55139)
  process.env.CHARIOX_HOME = join(home, "elsewhere")
  assert.throws(() => kernelUnixClient(), (error: unknown) => error instanceof CommandFailure && error.code === "kernel_unreachable")
}))
