// MP-07 / MP-08 / MP-11: packaging cannot perform enrollment or install a service.
import assert from "node:assert/strict"
import test from "node:test"
import { spawnSync } from "node:child_process"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { packageSetupLinux } from "./package-chariox-setup-linux.mjs"
test("MP-07/MP-11 unsigned Linux package contains only generic executable and public metadata", async t => {
  const dir = await mkdtemp(join(process.env.CHARIOX_BYOM_TEST_STATE ?? tmpdir(), "setup-deb-test-")); t.after(() => rm(dir, { recursive: true, force: true }))
  const binary = join(dir, "setup"), output = join(dir, "setup.deb")
  await writeFile(binary, "synthetic generic executable", { mode: 0o755 })
  await packageSetupLinux(binary, output, "0.3.0")
  assert.equal(spawnSync("dpkg-deb", ["--extract", output, join(dir, "extracted")]).status, 0)
  assert.deepEqual(await readFile(join(dir, "extracted/usr/bin/chariox-setup")), await readFile(binary))
  const control = spawnSync("dpkg-deb", ["--ctrl-tarfile", output])
  const names = spawnSync("tar", ["-tf", "-"], { input: control.stdout, encoding: "utf8" }).stdout.trim().split("\n")
  assert.deepEqual(names.sort(), ["./", "./control"])
  assert.equal(spawnSync("dpkg-deb", ["--field", output, "Architecture"], { encoding: "utf8" }).stdout.trim(), "amd64")
  await assert.rejects(packageSetupLinux(binary, output, "0.3.0\nmalformed"), /version/)
})
