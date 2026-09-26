import assert from "node:assert/strict"
import { spawn, type ChildProcess } from "node:child_process"
import { createECDH } from "node:crypto"
import {
  chmodSync,
  existsSync,
  linkSync,
  lstatSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  symlinkSync,
  unlinkSync,
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

test("identity recovery rejects an external hardlink and preserves unrelated temp-shaped files", (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-cli-relay-identity-external-link-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const identityPath = path.join(root, "relay", "cli-identity-v1.json")
  const store = createCliRelayIdentityStore(identityPath)
  const identity = store.getOrCreate()
  const externalPath = path.join(root, "external-identity-link")
  const unrelatedPath = path.join(
    path.dirname(identityPath),
    `.${path.basename(identityPath)}.tmp-${process.pid}-${"f".repeat(32)}`,
  )
  const unrelatedContents = "unrelated file that recovery must preserve"
  writeFileSync(unrelatedPath, unrelatedContents, { mode: 0o600 })
  linkSync(identityPath, externalPath)

  assert.equal(statSync(identityPath).nlink, 2)
  assert.throws(() => store.getOrCreate(), /single-link/)
  assert.equal(existsSync(externalPath), true)
  assert.equal(statSync(identityPath).nlink, 2)
  assert.equal(readFileSync(unrelatedPath, "utf8"), unrelatedContents)

  unlinkSync(externalPath)
  assert.equal(store.getOrCreate().publicKeyThumbprint, identity.publicKeyThumbprint)
})

test("identity recovery rejects more than two links without removing any link", (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-cli-relay-identity-many-links-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const identityPath = path.join(root, "relay", "cli-identity-v1.json")
  const store = createCliRelayIdentityStore(identityPath)
  const identity = store.getOrCreate()
  const externalPaths = [path.join(root, "external-link-a"), path.join(root, "external-link-b")]
  for (const externalPath of externalPaths) linkSync(identityPath, externalPath)

  assert.equal(statSync(identityPath).nlink, 3)
  assert.throws(() => store.load(), /single-link/)
  assert.equal(statSync(identityPath).nlink, 3)
  for (const externalPath of externalPaths) assert.equal(existsSync(externalPath), true)

  for (const externalPath of externalPaths) unlinkSync(externalPath)
  assert.equal(store.getOrCreate().publicKeyThumbprint, identity.publicKeyThumbprint)
})

test("identity loading rejects a symlink identity path without following its target", (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-cli-relay-identity-symlink-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const targetPath = path.join(root, "established-identity.json")
  const targetStore = createCliRelayIdentityStore(targetPath)
  const identity = targetStore.getOrCreate()
  const identityPath = path.join(root, "relay", "cli-identity-v1.json")
  mkdirSync(path.dirname(identityPath), { recursive: true, mode: 0o700 })
  symlinkSync(targetPath, identityPath)

  assert.throws(() => createCliRelayIdentityStore(identityPath).getOrCreate(), /could not be opened safely/)
  assert.equal(lstatSync(identityPath).isSymbolicLink(), true)
  assert.equal(statSync(targetPath).nlink, 1)
  assert.equal(targetStore.getOrCreate().publicKeyThumbprint, identity.publicKeyThumbprint)
})

test("identity recovery rejects a symlink temp alias without following or removing its target", (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-cli-relay-identity-temp-symlink-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const identityPath = path.join(root, "relay", "cli-identity-v1.json")
  const store = createCliRelayIdentityStore(identityPath)
  const identity = store.getOrCreate()
  const externalPath = path.join(root, "external-identity-link")
  const aliasPath = path.join(
    path.dirname(identityPath),
    `.${path.basename(identityPath)}.tmp-${process.pid}-${"e".repeat(32)}`,
  )
  linkSync(identityPath, externalPath)
  symlinkSync(externalPath, aliasPath)

  assert.equal(statSync(identityPath).nlink, 2)
  assert.throws(() => store.getOrCreate(), /single-link/)
  assert.equal(lstatSync(aliasPath).isSymbolicLink(), true)
  assert.equal(existsSync(externalPath), true)
  assert.equal(statSync(identityPath).nlink, 2)

  unlinkSync(aliasPath)
  unlinkSync(externalPath)
  assert.equal(store.getOrCreate().publicKeyThumbprint, identity.publicKeyThumbprint)
})

test("getOrCreate refuses to rotate a malformed established identity", (t) => {
  const root = mkdtempSync(path.join(os.tmpdir(), "chariox-cli-relay-identity-malformed-existing-"))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const identityPath = path.join(root, "relay", "cli-identity-v1.json")
  const store = createCliRelayIdentityStore(identityPath)
  store.getOrCreate()
  const malformedContents = "not-an-identity"
  writeFileSync(identityPath, malformedContents, { mode: 0o600 })

  assert.throws(() => store.getOrCreate(), /malformed/)
  assert.equal(readFileSync(identityPath, "utf8"), malformedContents)
  assert.equal(temporaryIdentityFiles(identityPath).length, 0)
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
