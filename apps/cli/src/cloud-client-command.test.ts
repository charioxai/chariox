import assert from "node:assert/strict"
import test from "node:test"
import { execFile } from "node:child_process"
import { promisify } from "node:util"
import { mkdtemp, readdir, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { createServer } from "node:net"
import { fileURLToPath } from "node:url"
import { createCloudClientCommands } from "./cloud-client-command.js"
import type { CloudClient } from "./cloud-client.js"

const execute = promisify(execFile)

test("the actual cloud status executable runs without connecting to or creating a kernel", async () => {
  const root = await mkdtemp(join(tmpdir(), "kauth-client-command-"))
  let connections = 0
  const trap = createServer(socket => {connections++; socket.destroy()})
  await new Promise<void>(resolve => trap.listen(0, "127.0.0.1", resolve))
  const address = trap.address(); assert.ok(address && typeof address === "object")
  try {
    const result = await execute("bun", [fileURLToPath(new URL("./index.js", import.meta.url)), "cloud", "status"], {env: {...process.env, CHARIOX_HOME: root, CHARIOX_KERNEL_PORT: String(address.port)}, timeout: 15_000})
    assert.ok(result.stdout.includes("Terminal is signed out"))
    assert.equal(connections, 0)
    const entries = await readdir(root)
    assert.ok(!entries.includes("kernels") && !entries.includes("daemon"))
  } finally {await new Promise<void>(resolve => trap.close(() => resolve())); await rm(root, {recursive: true, force: true})}
})

test("detached Login uses client enrollment without waiting for a kernel; kernel link remains separate", async () => {
  let logins = 0, stored = 0, opened = 0, refreshed = 0
  const client = {login: async (_url: string, show: (value: {verificationUrl: string; userCode: string}) => Promise<void>) => {logins++; await show({verificationUrl: "https://cloud.example.test/activate?user_code=PUBLIC", userCode: "PUBLIC"}); return {accountId: "account", accountSlug: "fixture", clientId: "client"}}, profile: async () => null} as unknown as CloudClient
  const handle = createCloudClientCommands({client, isKernelConnected: () => false, apiUrl: () => "https://cloud.example.test", notice: () => {}, saveProfile: async () => {stored++}, refresh: async () => {refreshed++}, openUrl: async () => {opened++; return true}})
  assert.equal(await handle([]), true)
  assert.equal(logins, 1); assert.equal(stored, 1); assert.equal(opened, 1); assert.equal(refreshed, 1)
  assert.equal(await handle(["link"]), false); assert.equal(logins, 1)
})
