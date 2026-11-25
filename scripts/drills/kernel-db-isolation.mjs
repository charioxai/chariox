import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { existsSync, statSync } from "node:fs"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import { drillFlags, startDrillKernel } from "./own-kernel.mjs"

const { need } = drillFlags(process.argv)
const binary = need("--kernel-binary")
const stateRoot = need("--state-root")
const evidence = need("--evidence-dir")
const sweepScript = fileURLToPath(new URL("../sweep-kernel-dbs.mjs", import.meta.url))

function sweep(...databases) {
  return spawnSync(process.execPath, [
    sweepScript, ...databases.flatMap(db => ["--kernel-db", db]), stateRoot,
  ], { encoding: "utf8" })
}

async function withKernel(label, check) {
  const kernel = await startDrillKernel({ label, binary, stateRoot, evidence: join(evidence, label) })
  const db = join(kernel.env.CHARIOX_HOME, "state", "kernel.db")
  try {
    assert.equal(statSync(kernel.dir).mode & 0o777, 0o700)
    assert.ok(statSync(db).isFile(), "normal startup created the kernel database")
    await check(kernel, db)
  } finally {
    await kernel.stop()
    assert.ok(!existsSync(kernel.dir), "run directory removed")
  }
}

await withKernel("db-source", async (source, sourceDb) => {
  const work = join(source.dir, "work")
  const { session } = (await source.rpc({
    CreateSession: { alias: "seeded-room", workspace_id: work, worktree_id: work },
  })).SessionCreated
  const { profile } = (await source.rpc({
    CreateProviderAccountProfile: { provider: "claude", label: "unsigned-seed" },
  })).ProviderAccountProfile
  assert.ok((await source.rpc({ ListSessions: null })).SessionsListed.sessions.some(room => room.id === session.id))
  assert.ok((await source.rpc({ ListProviderAccountProfiles: { provider: "claude" } }))
    .ProviderAccountProfilesListed.profiles.some(account => account.profile_id === profile.profile_id))

  await withKernel("db-fresh", async (fresh, freshDb) => {
    const rooms = (await fresh.rpc({ ListSessions: null })).SessionsListed.sessions
    const { profiles } = (await fresh.rpc({ ListProviderAccountProfiles: { provider: "claude" } })).ProviderAccountProfilesListed
    assert.equal(rooms.length, 0)
    assert.ok(!profiles.some(account => account.profile_id === profile.profile_id))
    console.log("DC1 fresh boot has no seeded Room or provider profile")

    const red = sweep(sourceDb)
    assert.equal(red.status, 1, red.stderr)
    assert.equal(red.stdout.trim(), freshDb)
    assert.equal(sweep(sourceDb, freshDb).status, 0)
    console.log("DC3 sweep reports the undeclared database and accepts exact configured paths")
  })
  assert.equal(sweep(sourceDb).status, 0)
})
assert.equal(sweep().status, 0)
console.log("DC2 both private run directories removed")

await assert.rejects(startDrillKernel({
  label: "db-setup-failure",
  binary,
  stateRoot,
  evidence: join(evidence, "db-setup-failure"),
  profilePath: join(stateRoot, "missing-profile"),
}), /link account profile.*No such file or directory/)
assert.equal(sweep().status, 0)
console.log("DC2 failed setup removed its private kernel database")
