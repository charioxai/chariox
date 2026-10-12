import assert from "node:assert/strict"
import { mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import type { CloudClient } from "./cloud-client.js"
import { createCloudWaitingRoomController } from "./cloud-waiting-room-controller.js"

test("detached waiting-room discovery is single-flight and cannot overwrite a completed kernel pivot", async () => {
  let release!: () => void, connected = false, calls = 0, writes = 0
  const pending = new Promise<void>(resolve => { release = resolve })
  const client = { profile: async () => ({accountId: "account"}), directory: async () => { calls++; await pending; return [{daemonId: "kernel", machineId: "machine", status: "ONLINE"}] } } as unknown as CloudClient
  const refresh = createCloudWaitingRoomController({client, isKernelConnected: () => connected, setMachines: () => {writes++}, setKernels: () => {writes++}, setStatus: () => {}, reconcile: () => {writes++}})
  const a = refresh(), b = refresh()
  await Promise.resolve(); await Promise.resolve()
  assert.equal(calls, 1)
  connected = true; release(); await Promise.all([a,b])
  assert.equal(writes, 0)
})

test("signed-out detached waiting room clears only directory data and never creates or logs in a kernel", async () => {
  let machines: unknown[] = ["stale"], kernels: unknown[] = ["stale"], status = ""
  const client = { profile: async () => null, directory: async () => {throw new Error("signed-out discovery cannot authenticate")} } as unknown as CloudClient
  const refresh = createCloudWaitingRoomController({client, isKernelConnected: () => false, setMachines: rows => {machines = rows}, setKernels: rows => {kernels = rows}, setStatus: value => {status = value}, reconcile: () => {}})
  await refresh()
  assert.equal(machines.length, 0); assert.equal(kernels.length, 0); assert.equal(status, "ready")
})


test("an absent profile resolving after attachment cannot clear the kernel directory", async () => {
  let release!: () => void, connected = false, writes = 0
  const pending = new Promise<void>(resolve => {release = resolve})
  const client = {profile: async () => {await pending; return null}} as unknown as CloudClient
  const refresh = createCloudWaitingRoomController({client, isKernelConnected: () => connected, setMachines: () => {writes++}, setKernels: () => {writes++}, setStatus: () => {writes++}, reconcile: () => {writes++}})
  const result = refresh(); connected = true; release(); await result
  assert.equal(writes, 0)
})

test("MP-08/MP-10 cold detached discovery waits for kernel information before ready", async () => {
  let release!: () => void
  const pending = new Promise<void>(resolve => { release = resolve })
  let status = "", connected = false, hydrated = 0
  const client = { profile: async () => ({ accountId: "account" }), directory: async () => [
    { daemonId: "kernel", machineId: "machine", status: "ONLINE" },
  ] } as unknown as CloudClient
  const refresh = createCloudWaitingRoomController({ client, isKernelConnected: () => connected,
    setMachines: () => {}, setKernels: () => {}, setStatus: value => { status = value }, reconcile: () => {},
    hydrateKernel: async () => { hydrated++; await pending; connected = true },
  })
  const result = refresh()
  for (let i = 0; i < 5; i++) await Promise.resolve()
  assert.equal(hydrated, 1)
  assert.equal(status, "loading")
  release(); await result
  assert.notEqual(status, "error")
})

// MP-08/MP-10: exercise actual connection admission, not just directory guards.
test("delayed automatic kernel A cannot replace a later fast user pivot B", async () => {
  // Admission creates the CLI relay identity; keep it out of the real CHARIOX_HOME.
  const root = await mkdtemp(join(tmpdir(), "waiting-room-connection-")), previous = process.env.CHARIOX_HOME
  process.env.CHARIOX_HOME = root
  try {
    const { createWaitingRoomKernelConnectionController } = await import("./waiting-room-kernel-client.js")
    const { createMutableLocalIpcClient } = await import("./mutable-local-ipc-client.js")
    let release!: () => void
    const delayed = new Promise<void>(resolve => { release = resolve })
    const closed: string[] = [], admitted: string[] = []
    const makeClient = (id: string) => ({ close: async () => { closed.push(id) } }) as unknown as import("./ipc.js").LocalIpcClient
    const original = makeClient("original"), a = makeClient("a"), b = makeClient("b")
    const client = createMutableLocalIpcClient(original)
    let connected = false, inventoryStarted = false
    const connection = createWaitingRoomKernelConnectionController({ client, clientId: "test",
      cloudClient: { profile: async () => ({}), connect: async (id: string) => id === "auto-a" ? a : b } as unknown as CloudClient,
      kernelConnected: () => connected, homeKernelId: () => null, homeMachineId: () => null, currentKernelId: () => null,
      getInventory: async target => {
        if (target === a) { inventoryStarted = true; await delayed }
        return { kernelId: target === a ? "auto-a" : "user-b", machineId: "machine" } as import("./waiting-room-inventory-api.js").WaitingRoomInventory
      }, invalidateInventory: () => {}, clearProjectSetup: () => {}, connected: id => { connected = true; admitted.push(id) },
    })
    const resultA = connection.connect("auto-a", "machine")
    while (!inventoryStarted) await new Promise(resolve => setImmediate(resolve))
    assert.equal(await connection.connect("user-b", "machine"), true)
    release()
    assert.equal(await resultA, false)
    assert.equal(client.currentClient(), b)
    assert.deepEqual(admitted, ["user-b"])
    assert.ok(closed.includes("a"))
  } finally {
    if (previous === undefined) delete process.env.CHARIOX_HOME; else process.env.CHARIOX_HOME = previous
    await rm(root, { recursive: true, force: true })
  }
})

// MP-08/MP-10: automatic hydration passes its selection lifetime to admission.
test("automatic hydration activity expires when user selection supersedes it", async () => {
  let generation = 0, attached = false, guard: (() => boolean) | undefined
  let release!: () => void
  const delayed = new Promise<void>(resolve => { release = resolve })
  const client = { profile: async () => ({}), directory: async () => [{ daemonId: "auto-a", machineId: "machine", status: "ONLINE" }] } as unknown as CloudClient
  const refresh = createCloudWaitingRoomController({ client, isKernelConnected: () => false,
    setMachines: () => {}, setKernels: () => {}, setStatus: () => {}, reconcile: () => {},
    captureActivity: () => { const captured = generation; return () => generation === captured && !attached },
    hydrateKernel: async (_kernel, _machine, isActive) => { guard = isActive; await delayed },
  })
  const result = refresh()
  for (let i = 0; i < 5; i++) await Promise.resolve()
  assert.equal(guard?.(), true)
  generation++; assert.equal(guard?.(), false)
  attached = true; assert.equal(guard?.(), false)
  release(); await result
})

test("BY-03: a directory refusal that ended the login shows the signed-out room, not a directory error", async () => {
  let signedOut = false, machines: unknown[] = ["stale"], kernels: unknown[] = ["stale"], status = "", reconciled = 0
  const client = {
    profile: async () => signedOut ? null : {accountId: "account"},
    directory: async () => { signedOut = true; throw new Error("signed_out") },
  } as unknown as CloudClient
  const refresh = createCloudWaitingRoomController({client, isKernelConnected: () => false, setMachines: rows => {machines = rows}, setKernels: rows => {kernels = rows}, setStatus: value => {status = value}, reconcile: () => {reconciled++}})
  await refresh()
  assert.deepEqual([machines, kernels, status], [[], [], "ready"])
  assert.equal(reconciled, 1)
})

test("a directory failure with the login intact is still a directory error", async () => {
  let status = ""
  const client = {profile: async () => ({accountId: "account"}), directory: async () => { throw new Error("offline") }} as unknown as CloudClient
  const refresh = createCloudWaitingRoomController({client, isKernelConnected: () => false, setMachines: () => {}, setKernels: () => {}, setStatus: value => {status = value}, reconcile: () => {}})
  await refresh()
  assert.equal(status, "error")
})

test("a kernel pivot completing during the error-path profile read leaves the attached kernel's directory alone", async () => {
  for (const second of [null, {accountId: "account"}]) {
    let release!: () => void, connected = false, writes = 0, reads = 0
    const pending = new Promise<void>(resolve => {release = resolve})
    const client = {profile: async () => { if (++reads === 1) return {accountId: "account"}; await pending; return second }, directory: async () => { throw new Error("signed_out") }} as unknown as CloudClient
    const refresh = createCloudWaitingRoomController({client, isKernelConnected: () => connected, setMachines: () => {writes++}, setKernels: () => {writes++}, setStatus: value => { if (value !== "loading") writes++ }, reconcile: () => {writes++}})
    const result = refresh()
    while (reads < 2) await new Promise(resolve => setTimeout(resolve, 1))
    connected = true; release(); await result
    assert.equal(writes, 0)
  }
})
