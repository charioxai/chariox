import { createECDH, randomBytes } from "node:crypto"
import {
  closeSync,
  constants as fsConstants,
  fchmodSync,
  fstatSync,
  fsyncSync,
  linkSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
  readdirSync,
  unlinkSync,
  writeFileSync,
} from "node:fs"
import path from "node:path"

import { RelayClientIdentity } from "@chariox/kernel-client/ipc"

import { preferencesPath } from "./preferences.js"

const IDENTITY_VERSION = 1
const MAX_IDENTITY_FILE_BYTES = 4 * 1024

type StoredCliRelayIdentity = {
  readonly version: number
  readonly private_key: string
}

export type CliRelayIdentityStore = {
  load(): RelayClientIdentity | null
  getOrCreate(): RelayClientIdentity
}

/** Stores a distinct CLI relay key outside workspaces with owner-only permissions. */
export function createCliRelayIdentityStore(
  identityPath = defaultCliRelayIdentityPath(),
): CliRelayIdentityStore {
  return {
    load: () => loadIdentity(identityPath),
    getOrCreate: () => loadIdentity(identityPath) ?? createIdentity(identityPath),
  }
}

export function defaultCliRelayIdentityPath(): string {
  return path.join(path.dirname(preferencesPath()), "relay", "cli-identity-v1.json")
}

function createIdentity(identityPath: string): RelayClientIdentity {
  const directory = path.dirname(identityPath)
  mkdirSync(directory, { recursive: true, mode: 0o700 })
  const directoryDescriptor = openSync(
    directory,
    fsConstants.O_RDONLY | fsConstants.O_DIRECTORY | fsConstants.O_NOFOLLOW,
  )
  let descriptor: number | null = null
  let temporaryPath: string | null = null
  let privateKey: Buffer | null = null
  let serialized: Buffer | null = null
  try {
    const metadata = fstatSync(directoryDescriptor)
    const currentUid = process.getuid?.()
    if (!metadata.isDirectory() || (currentUid !== undefined && metadata.uid !== currentUid)) {
      throw new Error("CLI relay identity directory must be an owned directory")
    }
    fchmodSync(directoryDescriptor, 0o700)
    removeAbandonedTemporaryFiles(identityPath, directoryDescriptor, currentUid)

    const ecdh = createECDH("prime256v1")
    ecdh.generateKeys()
    privateKey = Buffer.from(ecdh.getPrivateKey())
    serialized = Buffer.from(JSON.stringify({
      version: IDENTITY_VERSION,
      private_key: privateKey.toString("base64"),
    } satisfies StoredCliRelayIdentity), "utf8")
    const candidatePath = temporaryIdentityPath(identityPath)
    descriptor = openSync(
      candidatePath,
      fsConstants.O_WRONLY
        | fsConstants.O_CREAT
        | fsConstants.O_EXCL
        | fsConstants.O_NOFOLLOW,
      0o600,
    )
    temporaryPath = candidatePath
    fchmodSync(descriptor, 0o600)
    writeFileSync(descriptor, serialized)
    fsyncSync(descriptor)
    const completedDescriptor = descriptor
    closeSync(completedDescriptor)
    descriptor = null

    try {
      linkSync(temporaryPath, identityPath)
    } catch (error) {
      if (isAlreadyExists(error)) {
        const existing = loadIdentity(identityPath)
        if (existing) return existing
      }
      throw error
    }
    unlinkIfPresent(temporaryPath)
    temporaryPath = null
    fsyncSync(directoryDescriptor)
  } catch (error) {
    throw new Error(`CLI relay identity could not be created safely: ${String(error)}`)
  } finally {
    if (descriptor !== null) closeSync(descriptor)
    if (temporaryPath !== null) unlinkIfPresent(temporaryPath)
    closeSync(directoryDescriptor)
    privateKey?.fill(0)
    serialized?.fill(0)
  }

  const identity = loadIdentity(identityPath)
  if (!identity) throw new Error("CLI relay identity was not available after creation")
  return identity
}

function loadIdentity(identityPath: string): RelayClientIdentity | null {
  let descriptor: number
  try {
    descriptor = openSync(identityPath, fsConstants.O_RDONLY | fsConstants.O_NOFOLLOW)
  } catch (error) {
    if (isMissing(error)) return null
    throw new Error(`CLI relay identity could not be opened safely: ${String(error)}`)
  }
  try {
    let metadata = fstatSync(descriptor)
    const currentUid = process.getuid?.()
    assertIdentityFile(identityPath, metadata, currentUid)
    if (metadata.nlink !== 1) {
      removePublishedTemporaryLink(identityPath, descriptor, metadata, currentUid)
      metadata = fstatSync(descriptor)
      assertIdentityFile(identityPath, metadata, currentUid)
      if (metadata.nlink !== 1) throw invalidIdentityFileError()
    }
    const contents = readFileSync(descriptor, "utf8")
    let stored: StoredCliRelayIdentity
    try {
      stored = JSON.parse(contents) as StoredCliRelayIdentity
    } catch {
      throw new Error("CLI relay identity file is malformed")
    }
    if (stored.version !== IDENTITY_VERSION || typeof stored.private_key !== "string") {
      throw new Error("CLI relay identity file has an unsupported format")
    }
    const privateKey = Buffer.from(stored.private_key, "base64")
    if (
      privateKey.length !== 32
      || privateKey.toString("base64") !== stored.private_key
    ) {
      privateKey.fill(0)
      throw new Error("CLI relay identity private key is malformed")
    }
    try {
      return new RelayClientIdentity(privateKey)
    } finally {
      privateKey.fill(0)
    }
  } finally {
    closeSync(descriptor)
  }
}

function assertIdentityFile(
  identityPath: string,
  metadata: ReturnType<typeof fstatSync>,
  currentUid: number | undefined,
): void {
  if (
    !metadata.isFile()
    || (metadata.mode & 0o777) !== 0o600
    || (currentUid !== undefined && metadata.uid !== currentUid)
    || metadata.size <= 0
    || metadata.size > MAX_IDENTITY_FILE_BYTES
  ) {
    throw invalidIdentityFileError()
  }
  const pathMetadata = lstatSync(identityPath)
  if (
    !pathMetadata.isFile()
    || pathMetadata.isSymbolicLink()
    || pathMetadata.dev !== metadata.dev
    || pathMetadata.ino !== metadata.ino
  ) {
    throw new Error("CLI relay identity changed while it was being read")
  }
}

function removePublishedTemporaryLink(
  identityPath: string,
  descriptor: number,
  metadata: ReturnType<typeof fstatSync>,
  currentUid: number | undefined,
): void {
  if (metadata.nlink !== 2) throw invalidIdentityFileError()
  const directory = path.dirname(identityPath)
  const prefix = temporaryIdentityPrefix(identityPath)
  const aliases = readdirSync(directory).filter((name) => {
    if (!isTemporaryIdentityName(name, prefix)) return false
    const temporaryPath = path.join(directory, name)
    try {
      const temporaryMetadata = lstatSync(temporaryPath)
      return temporaryMetadata.isFile()
        && !temporaryMetadata.isSymbolicLink()
        && temporaryMetadata.dev === metadata.dev
        && temporaryMetadata.ino === metadata.ino
    } catch (error) {
      if (isMissing(error)) return false
      throw error
    }
  })
  if (aliases.length !== 1) {
    if (fstatSync(descriptor).nlink === 1) return
    throw invalidIdentityFileError()
  }

  const alias = aliases[0]
  if (!alias) {
    if (fstatSync(descriptor).nlink === 1) return
    throw invalidIdentityFileError()
  }
  const temporaryPath = path.join(directory, alias)
  let temporaryMetadata: ReturnType<typeof lstatSync>
  try {
    temporaryMetadata = lstatSync(temporaryPath)
  } catch (error) {
    if (isMissing(error) && fstatSync(descriptor).nlink === 1) return
    throw error
  }
  if (
    !temporaryMetadata.isFile()
    || temporaryMetadata.isSymbolicLink()
    || (temporaryMetadata.mode & 0o777) !== 0o600
    || (currentUid !== undefined && temporaryMetadata.uid !== currentUid)
    || temporaryMetadata.nlink !== 2
    || temporaryMetadata.size !== metadata.size
  ) {
    throw invalidIdentityFileError()
  }
  unlinkIfPresent(temporaryPath)
  const directoryDescriptor = openSync(
    directory,
    fsConstants.O_RDONLY | fsConstants.O_DIRECTORY | fsConstants.O_NOFOLLOW,
  )
  try {
    fsyncSync(directoryDescriptor)
  } finally {
    closeSync(directoryDescriptor)
  }
  if (fstatSync(descriptor).nlink !== 1) throw invalidIdentityFileError()
}

function removeAbandonedTemporaryFiles(
  identityPath: string,
  directoryDescriptor: number,
  currentUid: number | undefined,
): void {
  const directory = path.dirname(identityPath)
  const prefix = temporaryIdentityPrefix(identityPath)
  let removed = false
  for (const name of readdirSync(directory)) {
    if (!isTemporaryIdentityName(name, prefix)) continue
    const creatorPid = Number(name.slice(prefix.length).split("-", 1)[0])
    if (!Number.isSafeInteger(creatorPid) || creatorPid <= 0 || creatorPid === process.pid || isProcessRunning(creatorPid)) {
      continue
    }
    const temporaryPath = path.join(directory, name)
    let metadata: ReturnType<typeof lstatSync>
    try {
      metadata = lstatSync(temporaryPath)
    } catch (error) {
      if (isMissing(error)) continue
      throw error
    }
    if (
      !metadata.isFile()
      || metadata.isSymbolicLink()
      || (metadata.mode & 0o777) !== 0o600
      || (currentUid !== undefined && metadata.uid !== currentUid)
      || metadata.nlink !== 1
      || metadata.size > MAX_IDENTITY_FILE_BYTES
    ) {
      continue
    }
    unlinkIfPresent(temporaryPath)
    removed = true
  }
  if (removed) fsyncSync(directoryDescriptor)
}

function isProcessRunning(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  } catch (error) {
    return !error || typeof error !== "object" || !("code" in error && error.code === "ESRCH")
  }
}

function temporaryIdentityPath(identityPath: string): string {
  const name = `${temporaryIdentityPrefix(identityPath)}${process.pid}-${randomBytes(16).toString("hex")}`
  return path.join(path.dirname(identityPath), name)
}

function temporaryIdentityPrefix(identityPath: string): string {
  return `.${path.basename(identityPath)}.tmp-`
}

function isTemporaryIdentityName(name: string, prefix: string): boolean {
  const parts = name.slice(prefix.length).split("-")
  const [pid, suffix] = parts
  return name.startsWith(prefix)
    && parts.length === 2
    && /^[1-9]\d*$/.test(pid ?? "")
    && /^[a-f0-9]{32}$/.test(suffix ?? "")
}

function unlinkIfPresent(filePath: string): void {
  try {
    unlinkSync(filePath)
  } catch (error) {
    if (!isMissing(error)) throw error
  }
}

function invalidIdentityFileError(): Error {
  return new Error("CLI relay identity file must be an owned, bounded, single-link mode-0600 file")
}

function isMissing(error: unknown): boolean {
  return !!error && typeof error === "object" && "code" in error && error.code === "ENOENT"
}

function isAlreadyExists(error: unknown): boolean {
  return !!error && typeof error === "object" && "code" in error && error.code === "EEXIST"
}
