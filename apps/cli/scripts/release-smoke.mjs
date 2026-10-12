// Smoke-tests a compiled `chariox` release executable with neither Node nor Bun
// on its PATH, the way a release installation runs it:
//
//   node apps/cli/scripts/release-smoke.mjs EXECUTABLE VERSION
//
// MP-07 / MP-08 / MP-10 / MP-11: checks standalone CLI startup.
// Claude hooks are kernel-owned and tested in provider::claude::native_tui;
// they require Node on the provider execution machine.
import assert from "node:assert/strict"
import { execFileSync } from "node:child_process"
import { mkdtemp, realpath, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import path from "node:path"


const [executableArg, version] = process.argv.slice(2)
if (!executableArg || !version) throw new Error("usage: release-smoke.mjs EXECUTABLE VERSION")
const executable = await realpath(executableArg)
const scratch = await mkdtemp(path.join(await realpath(tmpdir()), "chariox-release-smoke-"))
// Only system tools; the release must not need Node or Bun.
const env = { PATH: scratch, HOME: scratch, LANG: "C.UTF-8" }
const run = (...args) => execFileSync(executable, args, { encoding: "utf8", env, timeout: 20_000 })

try {
  assert.equal(run("--version").trim(), `chariox ${version}`)
  assert.match(run("--help"), /^usage: /)

  process.stdout.write(`${JSON.stringify({ ok: true, executable, version })}\n`)
} finally {
  await rm(scratch, { recursive: true, force: true })
}
