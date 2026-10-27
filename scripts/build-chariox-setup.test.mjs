// MP-07 / MP-11: capture the generated entry, then execute real Setup argument admission.
import assert from "node:assert/strict"
import { execFile } from "node:child_process"
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import test from "node:test"
import { promisify } from "node:util"
const execute = promisify(execFile)

test("MP-07 / MP-11 generated Setup entry retains the actionable failure without a stack trace", async t => {
  const root = await mkdtemp(join(tmpdir(), "setup-entry-test-"))
  t.after(() => rm(root, {recursive: true, force: true}))
  const bin = join(root, "bin"), output = join(root, "output")
  await mkdir(bin)
  // Capture only the compiler input. Setup itself and its thrown diagnostic are real.
  await writeFile(join(bin, "bun"), '#!/bin/sh\nentry="$4"\nfor arg do output=$arg; done\ncp "$entry" "$output"\n', {mode: 0o755})
  await execute(process.execPath, [resolve("scripts/build-chariox-setup.mjs"), "--version", "0.1.0", "--public-key", "a".repeat(64), "--target", "linux-x64", "--output", output], {env: {...process.env, PATH: `${bin}:${process.env.PATH}`}})
  const result = await execute(process.execPath, [join(output, "chariox-setup"), "--invalid-option"]).catch(error => error)
  assert.equal(result.code, 1)
  assert.equal(result.stdout, "")
  assert.equal(result.stderr, "MP-07/MP-08/MP-11: Setup failed: invalid Setup arguments; enrollment codes belong on stdin\n")
  assert.doesNotMatch(result.stderr, /at runSetup|entry\.mjs/)
})
