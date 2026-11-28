// A first `pnpm start:cli` builds for minutes: say so before cargo starts, on stderr only.
import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { chmodSync, mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

const startCli = fileURLToPath(new URL("./start-cli.mjs", import.meta.url))

test("the first run names the build before cargo starts", () => {
  const dir = mkdtempSync(join(tmpdir(), "chx-start-cli-"))
  try {
    mkdirSync(join(dir, "bin"))
    writeFileSync(join(dir, "bin/cargo"), "#!/bin/sh\necho CARGO-STARTED >&2\nexit 1\n")
    chmodSync(join(dir, "bin/cargo"), 0o755)
    const run = spawnSync(process.execPath, [startCli, "guide"], { encoding: "utf8", env: { ...process.env, PATH: `${join(dir, "bin")}:${process.env.PATH}`, CARGO_TARGET_DIR: join(dir, "target") } })
    assert.equal(run.stdout, "")
    const notice = run.stderr.indexOf("Building chariox-cli first: this takes about 5 minutes once; later runs start at once.")
    assert.ok(notice >= 0, run.stderr)
    assert.ok(notice < run.stderr.indexOf("CARGO-STARTED"))
  } finally {
    rmSync(dir, { recursive: true, force: true })
  }
})
