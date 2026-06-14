import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./drill-failure-summary.mjs", import.meta.url))

test("failure summary max-depth limits manifest discovery", async () => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "arroba-failure-summary-"))
  const rootManifest = path.join(dir, "arroba-drill-failure.json")
  const nestedManifest = path.join(dir, ".artifacts", "run", "arroba-drill-failure.json")
  await writeManifest(rootManifest, failureManifest({ rootDir: dir, drill: "root" }))
  await writeManifest(nestedManifest, failureManifest({ rootDir: path.dirname(nestedManifest), drill: "nested" }))

  const shallow = await runSummary(["--find", dir, "--max-depth", "0", "--json"])
  const broad = await runSummary(["--find", dir, "--json"])

  assert.equal(shallow.total, 1)
  assert.deepEqual(shallow.failures.map((failure) => failure.source), [rootManifest])
  assert.equal(broad.total, 2)
  assert.deepEqual(broad.failures.map((failure) => failure.source), [nestedManifest, rootManifest].sort())

  await rm(dir, { recursive: true, force: true })
})

test("failure summary rejects invalid max-depth", async () => {
  await assert.rejects(
    () => execFile(process.execPath, [scriptPath, "--find", ".", "--max-depth", "nope"]),
    /--max-depth must be a non-negative integer/,
  )
})

async function runSummary(args) {
  const { stdout } = await execFile(process.execPath, [scriptPath, ...args])
  return JSON.parse(stdout)
}

async function writeManifest(file, manifest) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify(manifest)}\n`, "utf8")
}

function failureManifest({ rootDir, drill }) {
  return {
    schema: "arroba.drill.failure.v1",
    rootDir,
    failedAt: "2026-06-13T00:00:00.000Z",
    metadata: { drill },
    error: {
      name: "Error",
      message: "relay target stale",
      stack: null,
    },
  }
}
