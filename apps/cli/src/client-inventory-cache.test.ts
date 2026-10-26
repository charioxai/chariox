import { createComposerInventoryLoader } from "./composer-inventory-loader.js"
import assert from "node:assert/strict"
import test from "node:test"
import { mkdtemp, rm, stat } from "node:fs/promises"
import { join } from "node:path"
import { tmpdir } from "node:os"
import { readClientInventory, writeClientInventory, invalidateClientInventories } from "./client-inventory-cache.js"
test("MP-08 / MP-10 persistent inventories survive a reader restart and clear on owner switch", async () => {
  const home = await mkdtemp(join(tmpdir(), "panes-client-cache-")), previous = process.env.CHARIOX_HOME
  process.env.CHARIOX_HOME = home
  try {
    await writeClientInventory({ key: "kernel/provider/account", owner: "user-one", savedAt: 1, value: { models: ["public-model"] } })
    assert.deepEqual((await readClientInventory<{models: string[]}>("kernel/provider/account", "user-one"))?.value.models, ["public-model"])
    assert.equal((await stat(join(home, "client-cache/inventories.json"))).mode & 0o777, 0o600)
    assert.equal(await readClientInventory("other-kernel/provider/account", "user-one"), null)
    assert.equal(await readClientInventory("kernel/provider/account", "user-two"), null)
    assert.equal(await readClientInventory("kernel/provider/account", "user-one"), null)
    await writeClientInventory({ key: "kernel/provider/account", owner: "user-two", savedAt: 2, value: { models: ["new-model"] } })
    await invalidateClientInventories()
    assert.equal(await readClientInventory("kernel/provider/account", "user-two"), null)
  } finally {
    if (previous === undefined) delete process.env.CHARIOX_HOME; else process.env.CHARIOX_HOME = previous
    await rm(home, { recursive: true, force: true })
  }
})

test("MP-08 / MP-10 stale composer cache paints before reconciliation and rejects an invalidated write", async () => {
  const home = await mkdtemp(join(tmpdir(), "panes-client-stale-")), previous = process.env.CHARIOX_HOME
  process.env.CHARIOX_HOME = home
  try {
    await writeClientInventory({ key: "scope", owner: "user", savedAt: 0, value: { models: ["cached"] } })
    let finish!: (value: { models: string[] }) => void
    const load = createComposerInventoryLoader(async () => ({ key: "scope", owner: "user" }), () => new Promise<{models: string[]}>(resolve => { finish = resolve }))
    const value = await load()
    assert.deepEqual(value.models, ["cached"])
    assert.ok(value.reconciliation)
    await invalidateClientInventories()
    finish({ models: ["fresh"] })
    assert.deepEqual((await value.reconciliation)?.models, ["fresh"])
    assert.equal(await readClientInventory("scope", "user"), null)
  } finally {
    if (previous === undefined) delete process.env.CHARIOX_HOME; else process.env.CHARIOX_HOME = previous
    await rm(home, { recursive: true, force: true })
  }
})
