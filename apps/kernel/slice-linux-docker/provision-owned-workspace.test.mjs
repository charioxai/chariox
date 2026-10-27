import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

const provisioner = fileURLToPath(new URL("./provision-linux-docker-slice.sh", import.meta.url))
const workspaceVolume = "chariox-owned-workspace-fixture-workspace"

test("explicit deletion removes the slice's owned workspace volume", async () => {
  const { result, calls } = await destroy({ owned: true })
  assert.equal(result.status, 0, result.stderr)
  const home = calls.findIndex((args) => args.join(" ") === "volume rm chariox-owned-workspace-fixture-home")
  const workspace = calls.findIndex((args) => args.join(" ") === `volume rm ${workspaceVolume}`)
  assert.ok(home >= 0 && workspace > home, "home and then workspace volume must be removed")
})

test("deletion refuses a workspace volume owned by another slice", async () => {
  const { result, calls } = await destroy({ owned: true, ownerSlice: "slice-other" })
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /foreign owned workspace volume/)
  assert.equal(calls.some((args) => args.join(" ") === `volume rm ${workspaceVolume}`), false)
})

test("deletion without an owned workspace leaves workspace volumes alone", async () => {
  const { result, calls } = await destroy({ owned: false })
  assert.equal(result.status, 0, result.stderr)
  assert.equal(calls.some((args) => args[0] === "volume" && args.includes(workspaceVolume)), false)
})

async function destroy({ owned, ownerSlice = "slice-fixture" }) {
  const root = await mkdtemp(join(tmpdir(), "chariox-owned-workspace-"))
  try {
    const log = join(root, "docker.jsonl")
    await writeFile(join(root, "docker"), `#!/usr/bin/env node
const fs = require("node:fs");
const args = process.argv.slice(2);
fs.appendFileSync(process.env.CHARIOX_TEST_DOCKER_LOG, JSON.stringify(args) + "\\n");
if (args[0] === "info" || args[0] === "ps" || args[0] === "rm") process.exit(0);
if (args[0] === "volume" && args[1] === "inspect") {
  const format = args.includes("--format") ? args[args.indexOf("--format") + 1] : "";
  if (format.includes("owner-slice")) console.log(process.env.CHARIOX_TEST_OWNER_SLICE);
  else if (format.includes("owner-uid")) console.log("managed");
  process.exit(0);
}
if (args[0] === "volume" && args[1] === "rm") process.exit(0);
throw new Error("unexpected Docker call: " + args.join(" "));
`, { mode: 0o700 })
    const env = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith("CHARIOX_SLICE_")))
    const result = spawnSync("bash", [provisioner, "destroy"], {
      encoding: "utf8", timeout: 15_000,
      env: { ...env, PATH: `${root}:${env.PATH}`, TMPDIR: root,
        CHARIOX_TEST_DOCKER_LOG: log, CHARIOX_TEST_OWNER_SLICE: ownerSlice,
        CHARIOX_SLICE_NAME: "chariox-owned-workspace-fixture", CHARIOX_SLICE_ID: "slice-fixture",
        ...(owned ? { CHARIOX_SLICE_OWNED_WORKSPACE: "1" } : {}),
      },
    })
    assert.equal(result.error, undefined)
    const contents = await readFile(log, "utf8").catch((error) => { if (error.code === "ENOENT") return ""; throw error })
    return { result, calls: contents.trim() ? contents.trim().split("\n").map(JSON.parse) : [] }
  } finally {
    await rm(root, { recursive: true, force: true })
  }
}
