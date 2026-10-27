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

test("empty production Phase 1 App schema counts as state; a pre-Apps database does not", async t => {
  const root = await mkdtemp(join(tmpdir(), "chariox-apps-rollback-schema-"))
  t.after(() => rm(root, {recursive: true, force: true}))
  const database = join(root, "home/chariox/.chariox/state/kernel.db")
  await mkdir(dirname(database), {recursive: true})
  const sql = []
  for (const module of ["installation.rs", "managed_state/migration.rs", "managed_state/store.rs"]) {
    const source = await readFile(new URL(`../packages/app-runtime/src/${module}`, import.meta.url), "utf8")
    // Execute the production initializer's SQL, including its exact constraints.
    const statement = source.match(/execute_batch\(\s*"([\s\S]*?)",\s*\)/)?.[1]
    assert.ok(statement); assert.ok(!statement.includes("\\"))
    sql.push(statement)
  }
  const result = spawnSync("python3", ["-c", `import sqlite3,sys
c=sqlite3.connect(sys.argv[1]); c.execute('CREATE TABLE sessions(id TEXT)'); c.commit(); c.close()
`, database], {encoding: "utf8"})
  assert.equal(result.status, 0, result.stderr)
  assert.equal(detect(root), "absent", "pre-Apps tables alone do not trip the boundary")
  const initialized = spawnSync("python3", ["-c", `import sqlite3,sys
c=sqlite3.connect(sys.argv[1]); c.executescript(sys.stdin.read())
assert c.execute('SELECT COUNT(*) FROM app_installations').fetchone()[0] == 0
assert c.execute('SELECT COUNT(*) FROM app_state_migrations').fetchone()[0] == 0
c.commit(); c.close()
`, database], {input: sql.join("\n"), encoding: "utf8"})
  assert.equal(initialized.status, 0, initialized.stderr)
  assert.equal(detect(root), "present", "initialized App tables are a durable Phase 1 write, including with no rows")
})
