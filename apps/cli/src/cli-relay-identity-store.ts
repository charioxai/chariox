import { createECDH } from "node:crypto"
import {
  closeSync,
  constants as fsConstants,
  fchmodSync,
  fstatSync,
  fsyncSync,
  lstatSync,
  mkdirSync,
  openSync,
  readFileSync,
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
  try {
    const metadata = fstatSync(directoryDescriptor)
    const currentUid = process.getuid?.()
    if (!metadata.isDirectory() || (currentUid !== undefined && metadata.uid !== currentUid)) {
      throw new Error("CLI relay identity directory must be an owned directory")
    }
    fchmodSync(directoryDescriptor, 0o700)
  } finally {
    closeSync(directoryDescriptor)
  }

  const ecdh = createECDH("prime256v1")
  ecdh.generateKeys()
  const privateKey = Buffer.from(ecdh.getPrivateKey())
  const serialized = Buffer.from(JSON.stringify({
    version: IDENTITY_VERSION,
    private_key: privateKey.toString("base64"),
  } satisfies StoredCliRelayIdentity), "utf8")
  let descriptor: number
  try {
    descriptor = openSync(
      identityPath,
      fsConstants.O_WRONLY
        | fsConstants.O_CREAT
        | fsConstants.O_EXCL
        | fsConstants.O_NOFOLLOW,
      0o600,
    )
  } catch (error) {
    privateKey.fill(0)
    serialized.fill(0)
    if (isAlreadyExists(error)) {
      const existing = loadIdentity(identityPath)
      if (existing) return existing
    }
    throw new Error(`CLI relay identity could not be created safely: ${String(error)}`)
  }
  try {
    fchmodSync(descriptor, 0o600)
    writeFileSync(descriptor, serialized)
    fsyncSync(descriptor)
  } finally {
    closeSync(descriptor)
    privateKey.fill(0)
    serialized.fill(0)
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
    const metadata = fstatSync(descriptor)
    const currentUid = process.getuid?.()
    if (
      !metadata.isFile()
      || (metadata.mode & 0o077) !== 0
      || (currentUid !== undefined && metadata.uid !== currentUid)
      || metadata.nlink !== 1
      || metadata.size <= 0
      || metadata.size > MAX_IDENTITY_FILE_BYTES
    ) {
      throw new Error("CLI relay identity file must be an owned, bounded, single-link mode-0600 file")
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

function isMissing(error: unknown): boolean {
  return !!error && typeof error === "object" && "code" in error && error.code === "ENOENT"
}

function isAlreadyExists(error: unknown): boolean {
  return !!error && typeof error === "object" && "code" in error && error.code === "EEXIST"
}
