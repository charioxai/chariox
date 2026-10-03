import assert from "node:assert/strict"
import {spawnSync} from "node:child_process"
import {mkdir, mkdtemp, readFile, rm, symlink, writeFile} from "node:fs/promises"
import {tmpdir} from "node:os"
import {dirname, join} from "node:path"
import test from "node:test"

const detector = new URL("../deploy/managed-kernel/apps-rollback-state.py", import.meta.url).pathname
const detect = root => {
  const result = spawnSync("python3", [detector, root], {encoding: "utf8"})
  assert.equal(result.status, 0, result.stderr)
  return result.stdout.trim()
}
for (const kind of ["empty", "storage", "release", "tables", "corrupt", "redirected"]) {
  test(`Apps rollback state detection: ${kind}`, async t => {
    const root = await mkdtemp(join(tmpdir(), "chariox-apps-rollback-state-"))
    t.after(() => rm(root, {recursive: true, force: true}))
    const database = join(root, "home/chariox/.chariox/state/kernel.db")
    await mkdir(dirname(database), {recursive: true})
    const storage = join(root, "var/lib/chariox-app-storage")
    await mkdir(storage, {recursive: true})
    if (kind === "storage") await writeFile(join(storage, "synthetic-state"), "synthetic")
    if (kind === "release") {
      const releases = join(dirname(database), "app-releases-synthetic")
      await mkdir(releases); await writeFile(join(releases, "envelope.cxapp"), "synthetic")
    }
    if (kind === "tables") {
      const result = spawnSync("python3", ["-c", "import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute('CREATE TABLE app_state_migrations(id TEXT)'); c.commit(); c.close()", database])
      assert.equal(result.status, 0)
      const before = await readFile(database)
      assert.equal(detect(root), "present")
      assert.deepEqual(await readFile(database), before, "inspection cannot rewrite state")
    }
    if (kind === "corrupt") await writeFile(database, "synthetic invalid SQLite")
    if (kind === "redirected") await symlink("/nonexistent", database)
    assert.equal(detect(root), kind === "empty" ? "absent" : "present")
  })
}
