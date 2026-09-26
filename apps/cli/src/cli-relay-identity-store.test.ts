import assert from "node:assert/strict"
import { spawn, type ChildProcess } from "node:child_process"
import { createECDH } from "node:crypto"
import {
  chmodSync,
  existsSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { RelayClientIdentity } from "@chariox/kernel-client/ipc"

import { createCliRelayIdentityStore } from "./cli-relay-identity-store.js"

test("CLI relay identity is protected and persists the same non-exporting identity", (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-cli-relay-identity-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const identityPath = path.join(root, "relay", "cli-identity-v1.json")

  const first = createCliRelayIdentityStore(identityPath).getOrCreate()
  const second = createCliRelayIdentityStore(identityPath).load()
  assert.ok(second)
  assert.equal(second.publicKeyBase64, first.publicKeyBase64)
  assert.equal(second.publicKeyThumbprint, first.publicKeyThumbprint)
  assert.deepEqual(Object.keys(first).sort(), ["publicKeyBase64", "publicKeyThumbprint"])
  assert.equal("privateKey" in first, false)
  assert.equal(statSync(identityPath).mode & 0o777, 0o600)

  const stored = JSON.parse(readFileSync(identityPath, "utf8")) as { private_key: string }
  assert.equal(typeof stored.private_key, "string")
  const peer = createTestRelayIdentity()
  const encrypted = first.encrypt(peer.publicKeyBase64, "cli identity round trip")
  assert.equal(peer.decrypt(encrypted), "cli identity round trip")
  assert.equal(
    first.decrypt(peer.encrypt(first.publicKeyBase64, Buffer.from("persisted identity round trip"))),
    "persisted identity round trip",
  )
})

test("concurrent CLI relay identity creation publishes one complete winner", async (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-cli-relay-identity-race-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const identityPath = path.join(root, "relay", "cli-identity-v1.json")
  const readyPath = path.join(root, "writer-ready")
  const gatePath = path.join(root, "writer-gate")
  const preloadPath = createPublicationPreload(root, readyPath, gatePath)
  const first = startIdentityProcess(identityPath, preloadPath)
  const firstResult = collectChild(first)

  try {
    await waitForFile(readyPath)
    const secondResult = await collectChild(startIdentityProcess(identityPath))
    writeFileSync(gatePath, "publish")
    const firstCompletion = await firstResult

    assert.equal(secondResult.code, 0, secondResult.stderr)
    assert.equal(firstCompletion.code, 0, firstCompletion.stderr)
    assert.equal(firstCompletion.stdout, secondResult.stdout)
    const identity = createCliRelayIdentityStore(identityPath).load()
    assert.ok(identity)
    assert.equal(identity.publicKeyThumbprint, firstCompletion.stdout)
    const metadata = statSync(identityPath)
    assert.equal(metadata.mode & 0o777, 0o600)
    assert.equal(metadata.nlink, 1)
  } finally {
    writeFileSync(gatePath, "publish")
    if (first.exitCode === null && first.signalCode === null) {
      first.kill("SIGKILL")
      await firstResult
    }
  }
})

test("failed identity publication preserves a malformed winner and removes its temporary key", async (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-cli-relay-identity-failed-publish-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const identityPath = path.join(root, "relay", "cli-identity-v1.json")
  const readyPath = path.join(root, "writer-ready")
  const gatePath = path.join(root, "writer-gate")
  const preloadPath = createPublicationPreload(root, readyPath, gatePath)
  const writer = startIdentityProcess(identityPath, preloadPath)
  const writerResult = collectChild(writer)

  try {
    await waitForFile(readyPath)
    writeFileSync(identityPath, "not-an-identity", { mode: 0o600 })
    writeFileSync(gatePath, "publish")
    const result = await writerResult

    assert.notEqual(result.code, 0)
    assert.equal(readFileSync(identityPath, "utf8"), "not-an-identity")
    assert.equal(temporaryIdentityFiles(identityPath).length, 0)
  } finally {
    writeFileSync(gatePath, "publish")
    if (writer.exitCode === null && writer.signalCode === null) {
      writer.kill("SIGKILL")
      await writerResult
    }
  }
})

test("a writer killed before publication leaves no partial identity and its private temp is recoverable", async (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-cli-relay-identity-crash-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const identityPath = path.join(root, "relay", "cli-identity-v1.json")
  const readyPath = path.join(root, "writer-ready")
  const gatePath = path.join(root, "writer-gate")
  const preloadPath = createPublicationPreload(root, readyPath, gatePath, "kill-before-link")
  const writer = startIdentityProcess(identityPath, preloadPath)
  const result = await collectChild(writer)

  assert.equal(result.signal, "SIGKILL")
  assert.equal(existsSync(identityPath), false)
  assert.equal(temporaryIdentityFiles(identityPath).length, 1)

  const identity = createCliRelayIdentityStore(identityPath).getOrCreate()
  assert.equal(createCliRelayIdentityStore(identityPath).load()?.publicKeyThumbprint, identity.publicKeyThumbprint)
  assert.equal(temporaryIdentityFiles(identityPath).length, 0)
  assert.equal(statSync(identityPath).mode & 0o777, 0o600)
})

test("a reader safely completes publication when the creator pauses after its atomic link", async (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-cli-relay-identity-reader-race-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const identityPath = path.join(root, "relay", "cli-identity-v1.json")
  const readyPath = path.join(root, "writer-ready")
  const gatePath = path.join(root, "writer-gate")
  const preloadPath = createPublicationPreload(root, readyPath, gatePath, "pause-after-link")
  const writer = startIdentityProcess(identityPath, preloadPath)
  const writerResult = collectChild(writer)

  try {
    await waitForFile(readyPath)
    const identity = createCliRelayIdentityStore(identityPath).load()
    assert.ok(identity)
    assert.equal(statSync(identityPath).nlink, 1)
    assert.equal(temporaryIdentityFiles(identityPath).length, 0)
    writeFileSync(gatePath, "publish")

    const result = await writerResult
    assert.equal(result.code, 0, result.stderr)
    assert.equal(result.stdout, identity.publicKeyThumbprint)
  } finally {
    writeFileSync(gatePath, "publish")
    if (writer.exitCode === null && writer.signalCode === null) {
      writer.kill("SIGKILL")
      await writerResult
    }
  }
})

test("a writer killed immediately after publication leaves a complete recoverable identity", async (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-cli-relay-identity-published-crash-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const identityPath = path.join(root, "relay", "cli-identity-v1.json")
  const preloadPath = createPublicationPreload(root, "", "", "kill-after-link")
  const result = await collectChild(startIdentityProcess(identityPath, preloadPath))

  assert.equal(result.signal, "SIGKILL")
  assert.ok(statSync(identityPath).size > 0)
  assert.equal(statSync(identityPath).nlink, 2)
  const identity = createCliRelayIdentityStore(identityPath).load()
  assert.ok(identity)
  assert.equal(statSync(identityPath).nlink, 1)
  assert.equal(temporaryIdentityFiles(identityPath).length, 0)
})

test("CLI relay identity refuses a group-readable key file", (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-cli-relay-identity-mode-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const identityPath = path.join(root, "relay", "cli-identity-v1.json")
  createCliRelayIdentityStore(identityPath).getOrCreate()
  chmodSync(identityPath, 0o644)

  assert.throws(() => createCliRelayIdentityStore(identityPath).load(), /mode-0600/)
})

function createPublicationPreload(
  root: string,
  readyPath: string,
  gatePath: string,
  boundary: "pause-before-link" | "kill-before-link" | "pause-after-link" | "kill-after-link" = "pause-before-link",
): string {
  const preloadPath = path.join(root, "identity-publication-preload.mjs")
  writeFileSync(preloadPath, `
import fs from "node:fs"
import { syncBuiltinESMExports } from "node:module"

const originalWriteFileSync = fs.writeFileSync
const originalLinkSync = fs.linkSync
let paused = false
fs.writeFileSync = function (...args) {
  if (${JSON.stringify(boundary)} === "pause-before-link" && !paused && typeof args[0] === "number") {
    paused = true
    originalWriteFileSync.call(fs, ${JSON.stringify(readyPath)}, "ready")
    while (!fs.existsSync(${JSON.stringify(gatePath)})) {
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 10)
    }
  }
  return originalWriteFileSync.apply(fs, args)
}
fs.linkSync = function (...args) {
  if (${JSON.stringify(boundary)} === "kill-before-link") process.kill(process.pid, "SIGKILL")
  const result = originalLinkSync.apply(fs, args)
  if (${JSON.stringify(boundary)} === "pause-after-link") {
    originalWriteFileSync.call(fs, ${JSON.stringify(readyPath)}, "ready")
    while (!fs.existsSync(${JSON.stringify(gatePath)})) {
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 10)
    }
  }
  if (${JSON.stringify(boundary)} === "kill-after-link") process.kill(process.pid, "SIGKILL")
  return result
}
syncBuiltinESMExports()
`, { mode: 0o600 })
  return preloadPath
}

function startIdentityProcess(identityPath: string, preloadPath?: string): ChildProcess {
  const identityStoreUrl = new URL("./cli-relay-identity-store.js", import.meta.url).href
  const program = `
import { createCliRelayIdentityStore } from ${JSON.stringify(identityStoreUrl)}
const identity = createCliRelayIdentityStore(process.env.CLI_RELAY_IDENTITY_PATH).getOrCreate()
process.stdout.write(identity.publicKeyThumbprint)
`
  const args = [
    ...(preloadPath ? ["--import", preloadPath] : []),
    "--input-type=module",
    "-e",
    program,
  ]
  return spawn(process.execPath, args, {
    env: { ...process.env, CLI_RELAY_IDENTITY_PATH: identityPath },
    stdio: ["ignore", "pipe", "pipe"],
  })
}

function collectChild(child: ChildProcess): Promise<{
  code: number | null
  signal: NodeJS.Signals | null
  stdout: string
  stderr: string
}> {
  let stdout = ""
  let stderr = ""
  child.stdout?.setEncoding("utf8").on("data", (chunk: string) => { stdout += chunk })
  child.stderr?.setEncoding("utf8").on("data", (chunk: string) => { stderr += chunk })
  return new Promise((resolve, reject) => {
    child.once("error", reject)
    child.once("close", (code, signal) => resolve({ code, signal, stdout, stderr }))
  })
}

async function waitForFile(filePath: string): Promise<void> {
  const deadline = Date.now() + 10_000
  while (!existsSync(filePath)) {
    if (Date.now() >= deadline) throw new Error("identity writer did not reach its publication boundary")
    await new Promise((resolve) => setTimeout(resolve, 5))
  }
}

function temporaryIdentityFiles(identityPath: string): string[] {
  const directory = path.dirname(identityPath)
  const prefix = `.${path.basename(identityPath)}.tmp-`
  return readdirSync(directory).filter((name) => name.startsWith(prefix))
}

function createTestRelayIdentity(): RelayClientIdentity {
  const ecdh = createECDH("prime256v1")
  ecdh.generateKeys()
  const privateKey = ecdh.getPrivateKey()
  try {
    return new RelayClientIdentity(privateKey)
  } finally {
    privateKey.fill(0)
  }
}
