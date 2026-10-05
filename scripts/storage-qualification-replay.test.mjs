// MP-03/MP-10/MP-11: execute replay preflight without launching a kernel.
import test from "node:test"
import assert from "node:assert/strict"
import { mkdtemp, mkdir, writeFile, rm, symlink, access } from "node:fs/promises"
import { join } from "node:path"
import { spawnSync } from "node:child_process"

for (const disposition of ["separate", "equal", "descendant", "symlink descendant"]) {
  test(`MP-10: evidence/runtime preflight ${disposition}`, async t => {
    const root = await mkdtemp(join(process.env.CHARIOX_HOME ?? process.env.HOME, ".chariox-replay-path-test-"))
    t.after(() => rm(root, {recursive: true, force: true}))
    const bin = join(root, "bin"), runtime = join(root, "runtime"), marker = join(root, "launched")
    await mkdir(bin)
    await writeFile(join(bin, "id"), '#!/bin/sh\ncase "$1" in -u) echo 1000;; *) echo fixture-user;; esac\n', {mode: 0o755})
    await writeFile(join(bin, "node"), '#!/bin/sh\n: > "$HOME/launched"\n', {mode: 0o755})
    await symlink(runtime, join(root, "alias"))
    const evidence = disposition === "separate" ? join(root, "evidence") : disposition === "equal" ? runtime
      : disposition === "descendant" ? join(runtime, "evidence") : join(root, "alias", "evidence")
    const result = spawnSync("bash", [new URL("../deploy/local-linux/replay-storage-qualification.sh", import.meta.url).pathname],
      {encoding: "utf8", env: {PATH: `${bin}:/usr/bin:/bin`, HOME: root,
        CHARIOX_STORAGE_QUALIFICATION_ISOLATED_HOST: "1", M20_KERNEL_BINARY: process.execPath,
        M20_SLICE_IMAGE: `sha256:${"a".repeat(64)}`, M20_RUNTIME_ROOT: runtime, M20_ARTIFACT_DIR: evidence}})
    if (disposition === "separate") {
      assert.equal(result.status, 0, result.stderr)
      await access(marker)
    } else {
      assert.notEqual(result.status, 0, "cleanup must never own evidence")
      assert.match(result.stderr, /evidence.*runtime/)
      await assert.rejects(access(marker), {code: "ENOENT"})
    }
  })
}
