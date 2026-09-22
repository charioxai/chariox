import assert from "node:assert/strict"
import { createServer } from "node:net"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  childDiagnostics,
  machineMetadata,
  relayClaims,
  spawnObserved,
  waitForTcpListener,
} from "./live-room-environment-m1-drill.mjs"

test("M1 evidence reports truthful host platform and architecture metadata", async () => {
  assert.deepEqual(machineMetadata, {
    platform: os.platform(),
    arch: os.arch(),
  })
  assert.notEqual(machineMetadata.platform, "local development Mac")
  const source = await readFile(
    new URL("./live-room-environment-m1-drill.mjs", import.meta.url),
    "utf8",
  )
  assert.doesNotMatch(source, /machine:\s*["']local development Mac["']/)
})

test("M1 relay claims bind kernels to their machine and omit fabricated key claims", () => {
  const kernelClaims = relayClaims({
    subject: "room-environment-home-test",
    subjectKind: "kernel",
    actions: ["daemon_register"],
    userId: "user-1",
    machineId: "room-environment-home-machine-test",
  })
  assert.equal(kernelClaims.machine_id, "room-environment-home-machine-test")
  assert.equal(Object.hasOwn(kernelClaims, "public_key_thumbprint"), false)

  const clientClaims = relayClaims({
    subject: "client-user-1",
    subjectKind: "client",
    actions: ["client_connect"],
    userId: "user-1",
  })
  assert.equal(clientClaims.machine_id, null)
  assert.equal(Object.hasOwn(clientClaims, "public_key_thumbprint"), false)
})

test("M1 startup waits for a loopback relay listener", async () => {
  const server = createServer((socket) => socket.end())
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve))
  const address = server.address()
  assert.ok(address && typeof address === "object")
  try {
    const ready = await waitForTcpListener("127.0.0.1", address.port, 1_000)
    assert.deepEqual({ host: ready.host, port: ready.port }, { host: "127.0.0.1", port: address.port })
    assert.match(ready.at, /^\d{4}-\d{2}-\d{2}T/)
  } finally {
    await new Promise((resolve) => server.close(resolve))
  }
})

test("M1 startup records an unexpected child exit without secrets", async () => {
  const evidenceRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-room-m1-startup-test-"))
  try {
    const child = spawnObserved(
      "exit-fixture",
      process.execPath,
      process.env,
      evidenceRoot,
      ["-e", "process.exit(17)"],
    )
    await new Promise((resolve, reject) => {
      child.once("exit", resolve)
      child.once("error", reject)
    })
    const [diagnostic] = childDiagnostics([child])
    assert.equal(diagnostic.label, "exit-fixture")
    assert.equal(diagnostic.exit.code, 17)
    assert.equal(diagnostic.exit.signal, null)
    assert.equal(diagnostic.exit.expected, false)
    assert.equal(diagnostic.spawnError, null)
    assert.equal(diagnostic.outputLogPath, path.join(evidenceRoot, "exit-fixture.log"))
    assert.equal(diagnostic.runtimeLogDir, null)
    assert.equal(Object.hasOwn(diagnostic, "env"), false)
    assert.equal(Object.hasOwn(diagnostic, "token"), false)
  } finally {
    await rm(evidenceRoot, { recursive: true, force: true })
  }
})
