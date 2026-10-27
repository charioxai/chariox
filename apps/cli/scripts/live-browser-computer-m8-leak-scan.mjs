#!/usr/bin/env node

import { constants } from "node:fs"
import { createHash } from "node:crypto"
import { open, lstat, readdir, realpath, readFile } from "node:fs/promises"
import path from "node:path"
import { fileURLToPath } from "node:url"

import { redactDrillSecretText } from "./lib/drill-secrets.mjs"

export const M8_LEAK_SCAN_SCHEMA = "chariox.browser_computer_m8_leak_scan.v1"
export const M8_EVIDENCE_CATEGORIES = Object.freeze([
  "log",
  "history",
  "trace",
  "screenshot-metadata",
  "clipboard",
  "helper-output",
])

const MAX_CANARY_COUNT = 64
const MAX_CANARY_BYTES = 4_096
const MAX_CANARY_FILE_BYTES = 64 * 1_024
const MAX_ARTIFACT_FILES_PER_CATEGORY = 50_000
const MAX_ARTIFACT_BYTES_PER_CATEGORY = 1_073_741_824

function hashedIdentity(value) {
  return `sha256:${createHash("sha256").update(value).digest("hex")}`
}

function normalizeCanaries(values) {
  if (!Array.isArray(values) || values.length === 0 || values.length > MAX_CANARY_COUNT) {
    throw new Error("invalid canary input")
  }
  if (values.some((value) => typeof value !== "string" || value.length === 0
    || Buffer.byteLength(value, "utf8") > MAX_CANARY_BYTES)) {
    throw new Error("invalid canary input")
  }
  const unique = [...new Set(values)]
  if (unique.length !== values.length) throw new Error("invalid canary input")
  return unique.map((value) => ({ value, bytes: Buffer.from(value, "utf8"), id: hashedIdentity(value) }))
}

function emptyCategory(category, root = null) {
  return {
    category,
    root_id: root ? hashedIdentity(`root\0${category}\0${root}`) : null,
    available: false,
    missing: true,
    artifact_count: 0,
    bytes_scanned: 0,
    matching_artifact_count: 0,
    match_count: 0,
  }
}

export async function collectFiles(directory, { maxFiles = MAX_ARTIFACT_FILES_PER_CATEGORY } = {}) {
  const files = []
  async function visit(current, relative) {
    const entries = await readdir(current, { withFileTypes: true })
    for (const entry of entries.sort((left, right) => left.name.localeCompare(right.name))) {
      const absolute = path.join(current, entry.name)
      const childRelative = relative ? path.join(relative, entry.name) : entry.name
      if (entry.isSymbolicLink()) throw new Error("unavailable artifact class")
      if (entry.isDirectory()) {
        await visit(absolute, childRelative)
      } else if (entry.isFile()) {
        if (files.length >= maxFiles) throw new Error("unavailable artifact class")
        files.push({ absolute, relative: childRelative })
      } else {
        throw new Error("unavailable artifact class")
      }
    }
  }
  await visit(directory, "")
  return files
}

async function countCanaryBytes(handle, canaries) {
  const counts = canaries.map(() => 0)
  const longest = Math.max(...canaries.map(({ bytes }) => bytes.length))
  let carry = Buffer.alloc(0)
  let bytesScanned = 0
  const stream = handle.createReadStream({ autoClose: false })
  for await (const chunk of stream) {
    bytesScanned += chunk.length
    const data = carry.length === 0 ? chunk : Buffer.concat([carry, chunk])
    for (let index = 0; index < canaries.length; index += 1) {
      const needle = canaries[index].bytes
      let offset = 0
      let match = data.indexOf(needle, offset)
      while (match !== -1) {
        if (match + needle.length > carry.length) counts[index] += 1
        offset = match + 1
        match = data.indexOf(needle, offset)
      }
    }
    carry = longest > 1 ? Buffer.from(data.subarray(Math.max(0, data.length - longest + 1))) : Buffer.alloc(0)
  }
  return { counts, bytesScanned }
}

async function scanCategory(category, rootPath, canaries, seenRoots) {
  let report = emptyCategory(category, typeof rootPath === "string" ? path.resolve(rootPath) : null)
  const matches = []
  try {
    if (typeof rootPath !== "string" || !path.isAbsolute(rootPath)) return { report, matches }
    const initial = await lstat(rootPath)
    if (!initial.isDirectory() || initial.isSymbolicLink()) return { report, matches }
    const canonicalRoot = await realpath(rootPath)
    report = emptyCategory(category, canonicalRoot)
    if (seenRoots.has(canonicalRoot)) return { report, matches }
    seenRoots.add(canonicalRoot)

    const files = await collectFiles(canonicalRoot)
    report.artifact_count = files.length
    if (files.length === 0) return { report, matches }

    let expectedTotalBytes = 0n
    const fileStats = []
    for (const file of files) {
      const fileInfo = await lstat(file.absolute)
      if (!fileInfo.isFile() || fileInfo.isSymbolicLink() || fileInfo.size === 0) return { report, matches }
      expectedTotalBytes += BigInt(fileInfo.size)
      if (expectedTotalBytes > BigInt(MAX_ARTIFACT_BYTES_PER_CATEGORY)) return { report, matches }
      fileStats.push({ ...file, expected: fileInfo })
    }

    report.available = true
    const matchingArtifacts = new Set()
    for (const file of fileStats) {
      const flags = constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0)
      const handle = await open(file.absolute, flags)
      try {
        const before = await handle.stat()
        if (!before.isFile() || before.size !== file.expected.size || before.ino !== file.expected.ino) {
          report.available = false
          break
        }
        const { counts, bytesScanned } = await countCanaryBytes(handle, canaries)
        const after = await handle.stat()
        if (after.size !== before.size || after.ino !== before.ino || after.mtimeMs !== before.mtimeMs) {
          report.available = false
          break
        }
        report.bytes_scanned += bytesScanned
        const artifactId = hashedIdentity(`artifact\0${report.root_id}\0${file.relative.split(path.sep).join("/")}`)
        for (let index = 0; index < counts.length; index += 1) {
          if (counts[index] === 0) continue
          matchingArtifacts.add(artifactId)
          report.match_count += counts[index]
          matches.push({
            category,
            artifact_id: artifactId,
            canary_id: canaries[index].id,
            occurrence_count: counts[index],
          })
        }
      } finally {
        await handle.close()
      }
      if (!report.available) break
    }
    report.matching_artifact_count = matchingArtifacts.size
    report.missing = !report.available || report.bytes_scanned === 0
  } catch {
    report.available = false
    report.missing = true
  }
  return { report, matches }
}

function assertSafeReport(report, canaries) {
  const serialized = JSON.stringify(report)
  if (canaries.some(({ value }) => serialized.includes(value)) || redactDrillSecretText(serialized) !== serialized) {
    throw new Error("unsafe leak-scan report")
  }
  return report
}

export async function scanBrowserComputerM8Evidence({ canaryValues, artifactRoots } = {}) {
  const canaries = normalizeCanaries(canaryValues)
  if (!artifactRoots || typeof artifactRoots !== "object" || Array.isArray(artifactRoots)) {
    throw new Error("invalid artifact roots")
  }
  if (Object.keys(artifactRoots).some((category) => !M8_EVIDENCE_CATEGORIES.includes(category))) {
    throw new Error("invalid artifact roots")
  }

  const seenRoots = new Set()
  const results = []
  for (const category of M8_EVIDENCE_CATEGORIES) {
    results.push(await scanCategory(category, artifactRoots[category], canaries, seenRoots))
  }
  const categories = results.map(({ report }) => report)
  const matches = results.flatMap(({ matches: categoryMatches }) => categoryMatches)
  const leakCount = categories.reduce((total, category) => total + category.match_count, 0)
  const missingCategories = categories.filter((category) => category.missing).map((category) => category.category)
  const status = leakCount > 0 ? "fail" : missingCategories.length > 0 ? "incomplete" : "pass"
  return assertSafeReport({
    schema: M8_LEAK_SCAN_SCHEMA,
    status,
    ok: status === "pass",
    canary_count: canaries.length,
    canary_ids: canaries.map(({ id }) => id),
    categories,
    leak_count: leakCount,
    missing_categories: missingCategories,
    matches,
  }, canaries)
}

async function readCanaryValues(canaryFile) {
  if (typeof canaryFile !== "string" || !path.isAbsolute(canaryFile)) throw new Error("invalid canary file")
  const fileInfo = await lstat(canaryFile)
  if (!fileInfo.isFile() || fileInfo.isSymbolicLink() || (fileInfo.mode & 0o077) !== 0
    || fileInfo.size > MAX_CANARY_FILE_BYTES) throw new Error("invalid canary file")
  const parsed = JSON.parse(await readFile(canaryFile, "utf8"))
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)
    || Object.keys(parsed).length !== 1 || !Array.isArray(parsed.canary_values)) {
    throw new Error("invalid canary file")
  }
  return parsed.canary_values
}

function parseArgs(argv) {
  const result = { canaryFile: null, artifactRoots: {}, help: false }
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index]
    if (argument === "--help") {
      result.help = true
      continue
    }
    if (argument !== "--canaries-file" && argument !== "--artifact-root") throw new Error("invalid arguments")
    const value = argv[index + 1]
    if (!value || value.startsWith("--")) throw new Error("invalid arguments")
    index += 1
    if (argument === "--canaries-file") {
      if (result.canaryFile) throw new Error("invalid arguments")
      result.canaryFile = value
      continue
    }
    const separator = value.indexOf("=")
    if (separator < 1) throw new Error("invalid arguments")
    const category = value.slice(0, separator)
    const root = value.slice(separator + 1)
    if (!M8_EVIDENCE_CATEGORIES.includes(category) || !root || Object.hasOwn(result.artifactRoots, category)) {
      throw new Error("invalid arguments")
    }
    result.artifactRoots[category] = root
  }
  return result
}

function unavailableReport() {
  return {
    schema: M8_LEAK_SCAN_SCHEMA,
    status: "incomplete",
    ok: false,
    error_code: "invalid_input",
    canary_count: 0,
    canary_ids: [],
    categories: M8_EVIDENCE_CATEGORIES.map((category) => emptyCategory(category)),
    leak_count: 0,
    missing_categories: [...M8_EVIDENCE_CATEGORIES],
    matches: [],
  }
}

async function main(argv) {
  try {
    const options = parseArgs(argv)
    if (options.help) {
      process.stdout.write("Usage: live-browser-computer-m8-leak-scan.mjs --canaries-file PRIVATE_JSON --artifact-root CATEGORY=ABSOLUTE_PATH ...\n")
      return 0
    }
    if (!options.canaryFile) throw new Error("invalid arguments")
    const canaryValues = await readCanaryValues(options.canaryFile)
    const report = await scanBrowserComputerM8Evidence({ canaryValues, artifactRoots: options.artifactRoots })
    process.stdout.write(`${JSON.stringify(report)}\n`)
    return report.status === "pass" ? 0 : report.status === "fail" ? 1 : 2
  } catch {
    process.stdout.write(`${JSON.stringify(unavailableReport())}\n`)
    return 2
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  process.exitCode = await main(process.argv.slice(2))
}
