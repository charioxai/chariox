import assert from "node:assert/strict"
import test from "node:test"

import {
  waitingRoomRemoteKernelCanDelete,
  waitingRoomRemoteKernelIsAttachable,
  waitingRoomRemoteKernels,
  waitingRoomRemoteMachineCanDelete,
  waitingRoomRemoteRows,
} from "./waiting-room-remote-rows.js"

test("waiting room remote rows render relay status and cloud notices", () => {
  const rows = waitingRoomRemoteRows(
    { focus: "relay", machineIndex: 0, remoteKernelIndex: 0 },
    {
      cloudNotice: "Opening Cloud.\nurl=https://cloud.example",
      relay: { configured: true, connected: false, relay_url: "wss://relay.example" },
    },
    24,
  )

  assert.equal(rows.find((row) => row.id === "relay-header")?.value, "connecting wss://relay.example")
  assert.equal(rows.find((row) => row.id === "relay-configure")?.focused, true)
  assert.equal(rows.find((row) => row.id === "cloud-notice:0")?.value, "Opening Cloud.")
  assert.equal(rows.find((row) => row.id === "cloud-notice:1")?.value, "url=https://cloud.example")
  assert.equal(rows.find((row) => row.id === "machines-none")?.value, "waiting for relay connection")
})

test("waiting room remote rows render machine and kernel inventory", () => {
  const remote = {
    relay: { configured: true, connected: true, relay_url: "wss://relay.example" },
    machines: [{
      machine_id: "machine-1",
      machine_alias: "builder",
      display_name: "Builder",
      trust_status: "approved" as const,
      online: true,
      pending: false,
      kernel_count: 1,
      available_providers: ["codex"],
    }, {
      machine_id: "machine-2",
      display_name: "Cold",
      trust_status: "pending" as const,
      online: true,
      pending: true,
      kernel_count: 0,
      available_providers: [],
    }],
    kernels: [{
      kernel_id: "kernel-1",
      machine_id: "machine-1",
      machine_alias: "builder",
      relay_alias: "builder-kernel",
      accepting_remote_leases: true,
      leased_agent_count: 0,
      local_session_count: 1,
      available_providers: ["codex", "opencode"],
    }, {
      kernel_id: "kernel-2",
      machine_id: "machine-2",
      machine_alias: "cold",
      relay_alias: "cold-kernel",
      accepting_remote_leases: false,
      leased_agent_count: 0,
      local_session_count: 0,
      available_providers: [],
    }],
  }

  const rows = waitingRoomRemoteRows(
    { focus: "remote-kernel", machineIndex: 0, remoteKernelIndex: 0 },
    remote,
    24,
  )

  assert.equal(rows.find((row) => row.id === "machines-header")?.value, "2 online (1 pending)")
  assert.equal(rows.find((row) => row.id === "machine:machine-1")?.title, "Builder")
  assert.equal(rows.find((row) => row.id === "machine:machine-1")?.value, "1 kernel codex")
  assert.equal(rows.find((row) => row.id === "machine:machine-2")?.title, "Cold (pending)")
  assert.equal(rows.find((row) => row.id === "machine:machine-2")?.value, "0 kernels no providers · next: approve machine-2")
  assert.equal(rows.find((row) => row.id === "remote-kernel:kernel-1")?.title, "builder-kernel @ builder")
  assert.equal(rows.find((row) => row.id === "remote-kernel:kernel-1")?.value, "ready codex,opencode")
  assert.equal(rows.find((row) => row.id === "remote-kernel:kernel-1")?.focused, true)
  assert.equal(rows.find((row) => row.id === "remote-kernel:kernel-2")?.value, "inactive no providers · next: enable remote leases or choose another worker")
})

test("waiting room remote helpers classify deletable and attachable inventory", () => {
  assert.deepEqual(waitingRoomRemoteKernels({}), [])
  assert.equal(waitingRoomRemoteMachineCanDelete({
    machine_id: "machine-1",
    online: false,
    kernel_count: 1,
  }), true)
  assert.equal(waitingRoomRemoteKernelIsAttachable({
    kernel_id: "kernel-1",
    machine_id: "machine-1",
    accepting_remote_leases: false,
  }), false)
  assert.equal(waitingRoomRemoteKernelCanDelete({
    kernel_id: "kernel-1",
    machine_id: "machine-1",
    accepting_remote_leases: false,
    leased_agent_count: 0,
    local_session_count: 0,
  }), true)
})
