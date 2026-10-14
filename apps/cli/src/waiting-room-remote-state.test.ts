import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import test from "node:test"

for (const file of ["cli-input-routing-composition.ts", "cli-primary-transcript-composition.ts", "cli-app-root.tsx", "cli-automation-snapshot.ts"]) {
  test(`MP-08/MP-11 ${file} shares the home identity in its remote projection`, () => {
    const source = readFileSync(new URL(`../src/${file}`, import.meta.url), "utf8")
    assert.match(source, /import \{ waitingRoomRemoteTargets \} from "\.\/waiting-room-remote-state\.js"/)
    assert.match(source, /\.\.\.waitingRoomRemoteTargets\(/)
  })
}

test("MP-08/MP-11 shared projection keeps right-arrow navigation consistent with reconciliation", async () => {
  const { waitingRoomRemoteTargets } = await import("./waiting-room-remote-state.js")
  const { cycleWaitingRoomLaunchMachine, normalizeWaitingRoomLaunchPlacement } = await import("@chariox/kernel-client/waiting-room-runtime-placement")
  const remote = {
    ...waitingRoomRemoteTargets({ homeKernelId: "home", workspacePath: "/repo", worktreePath: "/repo", managedEnvironmentCatalog: { environments: [], computeClasses: [], contextSources: [] } }),
    relay: { connected: true, machine_id: "home-machine", daemon_id: "home" },
  }
  const next = cycleWaitingRoomLaunchMachine({ selectedMachineRef: "local", selectedKernelRef: "local" }, remote, 1)
  assert.equal(next.selectedMachineRef, "managed:new")
  assert.equal(normalizeWaitingRoomLaunchPlacement(next, remote).selectedMachineRef, "managed:new")
})
