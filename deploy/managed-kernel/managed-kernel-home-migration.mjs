#!/usr/bin/env node

import {
  chmod,
  chown,
  lstat,
  mkdir,
  open,
  readFile,
  rename,
  unlink,
} from "node:fs/promises"
import { basename, dirname, relative, resolve } from "node:path"

const SCHEMA_VERSION = 1
const MAX_JOURNAL_BYTES = 64 * 1024
const STATE_DIRECTORIES = ["managed-context", "managed-runtime-auth", "managed", "disposable-worker", "kernels"]
const CONTROL_ENTRIES = [
  ["managed/bootstrap-receipt.json", "managed/bootstrap-receipt.json", "file", "managed bootstrap receipt"],
  ["managed/release-override.json", "managed/release-override.json", "file", "managed release override"],
  ["disposable-worker/bootstrap-receipt.json", "disposable-worker/bootstrap-receipt.json", "file", "disposable worker bootstrap receipt"],
  ["disposable-worker/release-override.json", "disposable-worker/release-override.json", "file", "disposable worker release override"],
  ["kernels/active", "kernels/active", "directory", "managed kernel presence state"],
]

function fail(message) {
  throw new Error(message)
}

function exactKeys(value, expected) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false
  const actual = Object.keys(value).sort()
  const wanted = [...expected].sort()
  return actual.length === wanted.length && actual.every((key, index) => key === wanted[index])
}

async function optionalMetadata(path, options = undefined) {
  return lstat(path, options).catch((error) => {
    if (error.code === "ENOENT") return null
    throw error
  })
}

function identity(metadata) {
  return { dev: metadata.dev.toString(), ino: metadata.ino.toString() }
}

function sameIdentity(metadata, expected) {
  const actual = identity(metadata)
  return actual.dev === expected.dev && actual.ino === expected.ino
}

function owner(value, label) {
  const parsed = Number(value)
  if (!Number.isSafeInteger(parsed) || parsed < 0) fail(`${label} is invalid`)
  return parsed
}

function requireType(metadata, kind, label) {
  if (metadata.isSymbolicLink()
    || (kind === "directory" && !metadata.isDirectory())
    || (kind === "file" && !metadata.isFile())) {
    fail(`${label} is not a real ${kind}`)
  }
}

function inside(root, path, label) {
  const trustedRoot = resolve(root)
  const resolved = resolve(path)
  if (resolved !== trustedRoot && !resolved.startsWith(`${trustedRoot}/`)) {
    fail(`${label} escapes its trusted root`)
  }
  return resolved
}

async function requireRealAncestors(root, path, label) {
  const trustedRoot = resolve(root)
  const resolved = inside(trustedRoot, path, label)
  let current = trustedRoot
  const rootMetadata = await optionalMetadata(current)
  if (!rootMetadata || rootMetadata.isSymbolicLink() || !rootMetadata.isDirectory()) {
    fail(`${label} trusted root is invalid`)
  }
  for (const component of relative(trustedRoot, resolved).split("/").filter(Boolean).slice(0, -1)) {
    current = resolve(current, component)
    const metadata = await optionalMetadata(current)
    if (!metadata || metadata.isSymbolicLink() || !metadata.isDirectory()) {
      fail(`${label} ancestor is invalid`)
    }
  }
}

async function fsyncDirectory(path) {
  const handle = await open(path, "r")
  try {
    await handle.sync()
  } finally {
    await handle.close()
  }
}

async function writeExclusive(path, bytes, mode) {
  const handle = await open(path, "wx", mode)
  try {
    await handle.writeFile(bytes)
    await handle.sync()
  } finally {
    await handle.close()
  }
}

async function addEntry(entries, legacyHome, sources, destination, kind, label) {
  const existing = []
  for (const sourceRelative of sources) {
    const source = inside(legacyHome, resolve(legacyHome, sourceRelative), label)
    const metadata = await optionalMetadata(source, { bigint: true })
    if (!metadata) continue
    await requireRealAncestors(legacyHome, source, label)
    requireType(metadata, kind, label)
    existing.push({ sourceRelative, metadata })
  }
  if (existing.length > 1) fail(`${label} has more than one migration source`)
  if (existing.length === 0) return
  if (await optionalMetadata(destination)) fail(`${label} migration would overwrite an existing destination`)
  const [{ sourceRelative, metadata }] = existing
  entries.push({
    sourceRelative,
    destination: resolve(destination),
    kind,
    label,
    ...identity(metadata),
  })
}

async function plan(journalPath, roots, charioxUid, charioxGid) {
  const stateMetadata = await optionalMetadata(roots.stateRoot, { bigint: true })
  if (!stateMetadata) fail("managed kernel state root is missing")
  requireType(stateMetadata, "directory", "managed kernel state root")
  const legacyMetadata = await optionalMetadata(roots.legacyHome, { bigint: true })
  const managedMetadata = await optionalMetadata(roots.managedHome, { bigint: true })
  if (legacyMetadata) requireType(legacyMetadata, "directory", "legacy managed kernel home")
  if (managedMetadata) requireType(managedMetadata, "directory", "managed service-account home")
  if (legacyMetadata && managedMetadata) {
    fail("legacy managed kernel home and /home/chariox both exist; refusing to overwrite either")
  }

  const entries = []
  if (legacyMetadata) {
    for (const [source, destination, kind, label] of CONTROL_ENTRIES) {
      await addEntry(
        entries,
        roots.legacyHome,
        [source, `.chariox/${source}`],
        resolve(roots.stateRoot, destination),
        kind,
        label,
      )
    }
    for (const name of STATE_DIRECTORIES) {
      await addEntry(
        entries,
        roots.legacyHome,
        [name],
        resolve(roots.managedState, name),
        "directory",
        `managed kernel ${name}`,
      )
    }
  }

  const journal = {
    schemaVersion: SCHEMA_VERSION,
    ...roots,
    charioxUid,
    charioxGid,
    required: Boolean(legacyMetadata),
    rootIdentity: legacyMetadata ? identity(legacyMetadata) : null,
    entries,
  }
  await writeExclusive(journalPath, Buffer.from(`${JSON.stringify(journal, null, 2)}\n`), 0o600)
  await fsyncDirectory(dirname(journalPath))
}

function validateJournal(journal, expected) {
  if (!exactKeys(journal, [
    "schemaVersion", "stateRoot", "legacyHome", "managedHome", "managedState",
    "charioxUid", "charioxGid", "required", "rootIdentity", "entries",
  ]) || journal.schemaVersion !== SCHEMA_VERSION
    || typeof journal.required !== "boolean"
    || !Array.isArray(journal.entries)
    || journal.entries.length > CONTROL_ENTRIES.length + STATE_DIRECTORIES.length
    || Object.entries(expected).some(([key, value]) => journal[key] !== value)) {
    fail("managed home migration journal is invalid")
  }
  const validIdentity = (value) => value && exactKeys(value, ["dev", "ino"])
    && /^\d+$/.test(value.dev) && /^\d+$/.test(value.ino)
  if (journal.required !== Boolean(journal.rootIdentity)
    || (journal.rootIdentity && !validIdentity(journal.rootIdentity))) {
    fail("managed home migration root identity is invalid")
  }
  const sources = new Set()
  const destinations = new Set()
  for (const entry of journal.entries) {
    if (!exactKeys(entry, ["sourceRelative", "destination", "kind", "label", "dev", "ino"])
      || !["file", "directory"].includes(entry.kind)
      || typeof entry.label !== "string" || !entry.label
      || typeof entry.sourceRelative !== "string" || !entry.sourceRelative
      || entry.sourceRelative.startsWith("/") || entry.sourceRelative.split("/").includes("..")
      || !/^\d+$/.test(entry.dev) || !/^\d+$/.test(entry.ino)) {
      fail("managed home migration entry is invalid")
    }
    const source = inside(journal.managedHome, resolve(journal.managedHome, entry.sourceRelative), entry.label)
    const destination = resolve(entry.destination)
    if ((!destination.startsWith(`${journal.stateRoot}/`)
        && !destination.startsWith(`${journal.managedState}/`))
      || sources.has(source) || destinations.has(destination)) {
      fail("managed home migration entry path is invalid")
    }
    sources.add(source)
    destinations.add(destination)
  }
}

async function ensureDirectory(path, ownerValue, mode, label) {
  const metadata = await optionalMetadata(path)
  if (metadata) {
    requireType(metadata, "directory", label)
    return
  }
  await mkdir(path, { mode })
  await chown(path, ownerValue.uid, ownerValue.gid)
  await chmod(path, mode)
  await fsyncDirectory(dirname(path))
}

async function apply(journalPath, roots, charioxUid, charioxGid) {
  const journalMetadata = await lstat(journalPath)
  if (journalMetadata.isSymbolicLink() || !journalMetadata.isFile()
    || journalMetadata.size > MAX_JOURNAL_BYTES) {
    fail("managed home migration journal is invalid")
  }
  let journal
  try {
    journal = JSON.parse(await readFile(journalPath, "utf8"))
  } catch {
    fail("managed home migration journal is invalid JSON")
  }
  validateJournal(journal, { ...roots, charioxUid, charioxGid })
  requireType(await lstat(roots.stateRoot), "directory", "managed kernel state root")
  await ensureDirectory(
    dirname(roots.managedHome),
    { uid: 0, gid: 0 },
    0o755,
    "managed service-account home parent",
  )

  if (journal.required) {
    const legacyMetadata = await optionalMetadata(roots.legacyHome, { bigint: true })
    const managedMetadata = await optionalMetadata(roots.managedHome, { bigint: true })
    if (legacyMetadata) {
      requireType(legacyMetadata, "directory", "legacy managed kernel home")
      if (!sameIdentity(legacyMetadata, journal.rootIdentity)) {
        fail("legacy managed kernel home identity changed during migration")
      }
      if (managedMetadata) fail("managed service-account home obstructs migration")
      await rename(roots.legacyHome, roots.managedHome)
      await fsyncDirectory(dirname(roots.legacyHome))
      await fsyncDirectory(dirname(roots.managedHome))
    } else {
      if (!managedMetadata) fail("managed service-account home disappeared during migration")
      requireType(managedMetadata, "directory", "managed service-account home")
      if (!sameIdentity(managedMetadata, journal.rootIdentity)) {
        fail("managed service-account home identity changed during migration")
      }
    }
  } else {
    await ensureDirectory(
      roots.managedHome,
      { uid: charioxUid, gid: charioxGid },
      0o700,
      "managed service-account home",
    )
  }

  await ensureDirectory(
    roots.managedState,
    { uid: charioxUid, gid: charioxGid },
    0o700,
    "managed kernel state directory",
  )
  for (const path of [roots.managedHome, roots.managedState]) {
    await chown(path, charioxUid, charioxGid)
    await chmod(path, 0o700)
  }

  for (const entry of journal.entries) {
    const source = resolve(roots.managedHome, entry.sourceRelative)
    const destination = resolve(entry.destination)
    const sourceMetadata = await optionalMetadata(source, { bigint: true })
    const destinationMetadata = await optionalMetadata(destination, { bigint: true })
    if (sourceMetadata) {
      await requireRealAncestors(roots.managedHome, source, entry.label)
      requireType(sourceMetadata, entry.kind, entry.label)
      if (!sameIdentity(sourceMetadata, entry)) fail(`${entry.label} identity changed during migration`)
      if (destinationMetadata) fail(`${entry.label} migration destination became obstructed`)
      const parent = dirname(destination)
      const userState = parent === roots.managedState || parent.startsWith(`${roots.managedState}/`)
      await ensureDirectory(
        parent,
        userState ? { uid: charioxUid, gid: charioxGid } : { uid: 0, gid: 0 },
        0o700,
        `${entry.label} destination parent`,
      )
      await rename(source, destination)
      await fsyncDirectory(dirname(source))
      await fsyncDirectory(dirname(destination))
    } else {
      if (!destinationMetadata) fail(`${entry.label} disappeared during migration`)
      const destinationRoot = destination.startsWith(`${roots.managedState}/`)
        ? roots.managedState : roots.stateRoot
      await requireRealAncestors(destinationRoot, destination, entry.label)
      requireType(destinationMetadata, entry.kind, entry.label)
      if (!sameIdentity(destinationMetadata, entry)) {
        fail(`${entry.label} destination identity changed during migration`)
      }
    }
  }

  const completionPath = resolve(dirname(journalPath), "home-migration-complete")
  const completionMetadata = await optionalMetadata(completionPath)
  if (completionMetadata) {
    if (completionMetadata.isSymbolicLink() || !completionMetadata.isFile()
      || await readFile(completionPath, "utf8") !== "complete\n") {
      fail("managed home migration completion marker is invalid")
    }
  } else {
    await writeExclusive(completionPath, Buffer.from("complete\n"), 0o600)
    await fsyncDirectory(dirname(completionPath))
  }
}

async function run(args) {
  const [operation, journalPath, stateRoot, legacyHome, managedHome, managedState, uid, gid] = args
  if (!operation || !journalPath || !stateRoot || !legacyHome || !managedHome || !managedState
    || uid === undefined || gid === undefined || args.length !== 8) {
    fail("usage: managed-kernel-home-migration.mjs <plan|apply> <journal> <state-root> <legacy-home> <managed-home> <managed-state> <uid> <gid>")
  }
  const roots = {
    stateRoot: resolve(stateRoot),
    legacyHome: resolve(legacyHome),
    managedHome: resolve(managedHome),
    managedState: resolve(managedState),
  }
  const charioxUid = owner(uid, "managed service-account uid")
  const charioxGid = owner(gid, "managed service-account gid")
  if (operation === "plan") {
    await plan(resolve(journalPath), roots, charioxUid, charioxGid)
  } else if (operation === "apply") {
    await apply(resolve(journalPath), roots, charioxUid, charioxGid)
  } else {
    fail("unsupported managed home migration operation")
  }
}

try {
  await run(process.argv.slice(2))
} catch (error) {
  const message = error instanceof Error ? error.message : String(error)
  process.stderr.write(`${basename(process.argv[1])}: ${message}\n`)
  process.exitCode = 1
}
