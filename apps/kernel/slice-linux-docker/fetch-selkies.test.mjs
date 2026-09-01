import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

const command = fileURLToPath(new URL("./fetch-selkies.mjs", import.meta.url))

test("an unsupported runtime architecture is rejected before downloading", () => {
  const result = spawnSync(process.execPath, [command, "runtime", "/unused-selkies-output", "riscv64"], { encoding: "utf8" })
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /usage: fetch-selkies/)
  assert.doesNotMatch(result.stdout, /verified/)
})

test("a corrupt cached artifact is rejected and left intact for inspection", async () => {
  const directory = await mkdtemp(join(tmpdir(), "chariox-selkies-fetch-"))
  try {
    const artifact = join(directory, "selkies.tar.gz")
    await writeFile(artifact, "corrupt cached archive")
    const result = spawnSync(process.execPath, [command, "source", directory], { encoding: "utf8" })
    assert.notEqual(result.status, 0)
    assert.match(result.stderr, /cached selkies.tar.gz failed SHA-256 verification/)
    assert.equal(await readFile(artifact, "utf8"), "corrupt cached archive")
  } finally {
    await rm(directory, { recursive: true, force: true })
  }
})
