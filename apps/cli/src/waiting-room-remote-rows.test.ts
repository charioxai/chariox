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
  assert.equal(rows.find((row) => row.id === "machine:machine-1")?.value, "1 kernel codex · ready=1/1 leased=0")
  assert.equal(rows.find((row) => row.id === "machine:machine-2")?.title, "Cold (pending)")
  assert.equal(rows.find((row) => row.id === "machine:machine-2")?.value, "0 kernels no providers · next: approve machine-2")
  assert.equal(rows.find((row) => row.id === "remote-kernel:kernel-1")?.title, "builder-kernel @ builder")
  assert.equal(rows.find((row) => row.id === "remote-kernel:kernel-1")?.value, "ready codex,opencode")
  assert.equal(rows.find((row) => row.id === "remote-kernel:kernel-1")?.focused, true)
  assert.equal(rows.find((row) => row.id === "remote-kernel:kernel-2")?.value, "blocked no providers · next: enable remote leases on cold-kernel or choose another worker")
})

test("waiting room remote rows warn when machine kernels reject leases", () => {
  const rows = waitingRoomRemoteRows(
    { focus: "machine", machineIndex: 0, remoteKernelIndex: 0 },
    {
      relay: { configured: true, connected: true },
      machines: [{
        machine_id: "machine-1",
        display_name: "Worker",
        trust_status: "approved" as const,
        online: true,
        pending: false,
        kernel_count: 1,
        available_providers: ["codex"],
      }],
      kernels: [{
        kernel_id: "kernel-1",
        machine_id: "machine-1",
        accepting_remote_leases: false,
        leased_agent_count: 0,
        local_session_count: 0,
        available_providers: ["codex"],
      }],
    },
    24,
  )

  assert.equal(rows.find((row) => row.id === "machine:machine-1")?.value, "1 kernel codex · ready=0/1 blocked=1 leased=0 · next: enable remote leases on kernel-1 or choose another worker")
})

test("waiting room remote rows distinguish provider and unknown readiness", () => {
  const rows = waitingRoomRemoteRows(
    { focus: "remote-kernel", machineIndex: 0, remoteKernelIndex: 0 },
    {
      relay: { configured: true, connected: true },
      machines: [{
        machine_id: "machine-1",
        display_name: "Worker",
        trust_status: "approved" as const,
        online: true,
        pending: false,
        kernel_count: 2,
        available_providers: [],
      }],
      kernels: [{
        kernel_id: "kernel-provider",
        machine_id: "machine-1",
        relay_alias: "provider-kernel",
        accepting_remote_leases: true,
        available_providers: [],
      }, {
        kernel_id: "kernel-unknown",
        machine_id: "machine-1",
        relay_alias: "unknown-kernel",
        available_providers: [],
      }],
    },
    24,
  )

  assert.equal(rows.find((row) => row.id === "machine:machine-1")?.value, "2 kernels no providers · ready=0/2 needs-provider=1 unknown=1 leased=0 · next: fix listed kernel readiness issues on Worker or choose another worker")
  assert.equal(rows.find((row) => row.id === "remote-kernel:kernel-provider")?.value, "needs-provider no providers · next: configure provider CLIs on provider-kernel")
  assert.equal(rows.find((row) => row.id === "remote-kernel:kernel-unknown")?.value, "unknown no providers · next: refresh unknown-kernel readiness or reconnect that worker before launching remote agents")
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
