import { spawnOwned, signalOwnedProcessGroup } from "../../../kernel/slice-linux-docker/owned-process-signals.mjs"
import assert from "node:assert/strict"
import { once } from "node:events"
import test from "node:test"
import { createRunner, matchingProcesses } from "./local-rust-fault-drill-runtime.mjs"

test("controller cleanup distinguishes simultaneous drills by their owned process group", async () => {
  const marker = `chariox-fixture-ownership-${process.pid}`
  const children = [0, 1].map(() => spawnOwned(process.execPath, ["-e", "setInterval(() => {}, 1000)", marker], {
    detached: true, stdio: "ignore",
  }))
  try {
    await Promise.all(children.map((child) => once(child, "spawn")))
    const run = createRunner({ repoRoot: process.cwd(), children: new Set() })
    assert.deepEqual(await matchingProcesses(run, marker, children[0].pid), [String(children[0].pid)])
    assert.equal(children[1].exitCode, null)
  } finally {
    const exits = children.map((child) => once(child, "exit"))
    for (const child of children) signalOwnedProcessGroup(child, "SIGTERM")
    await Promise.all(exits)
  }
})
