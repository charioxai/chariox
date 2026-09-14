import assert from "node:assert/strict"
import { chmod, mkdtemp, rm, symlink, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { test } from "node:test"

import {
  parseManagedBrowserComputerParityArgs,
  readPrivateManagedParityConfig,
} from "./managed-browser-computer-parity-cli.mjs"

test("managed parity CLI accepts only metadata paths and external evidence", () => {
  const options = parseManagedBrowserComputerParityArgs([
    "--config", "/private/config.json",
    "--transport-module", "/reviewed/product-transport.mjs",
    "--evidence-root", "/evidence/cha-16",
  ], { repoRoot: "/source/chariox" })
  assert.equal(options.configPath, "/private/config.json")
  assert.equal(options.transportModulePath, "/reviewed/product-transport.mjs")
  assert.equal(options.evidenceRoot, "/evidence/cha-16")
  assert.throws(
    () => parseManagedBrowserComputerParityArgs([
      "--config", "/private/config.json",
      "--transport-module", "/reviewed/product-transport.mjs",
      "--evidence-root", "/source/chariox/evidence",
    ], { repoRoot: "/source/chariox" }),
    /outside the repository/,
  )
  assert.throws(
    () => parseManagedBrowserComputerParityArgs(["--token", "must-not-be-an-argument"], { repoRoot: "/source/chariox" }),
    /unknown managed parity argument/,
  )
})

test("managed parity CLI reads only private regular config files", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-managed-parity-cli-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const configPath = path.join(root, "config.json")
  await writeFile(configPath, '{"runId":"run-1"}\n', { mode: 0o600 })
  assert.deepEqual(await readPrivateManagedParityConfig(configPath), { runId: "run-1" })

  await chmod(configPath, 0o644)
  await assert.rejects(() => readPrivateManagedParityConfig(configPath), /must not be accessible/)
  await chmod(configPath, 0o600)
  const alias = path.join(root, "config-alias.json")
  await symlink(configPath, alias)
  await assert.rejects(() => readPrivateManagedParityConfig(alias), /regular file/)
})
