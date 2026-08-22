import assert from "node:assert/strict"
import test from "node:test"

import {
  cycleWaitingRoomLaunchKernel,
  cycleWaitingRoomLaunchMachine,
  formatWaitingRoomLaunchKernelValue,
  formatWaitingRoomLaunchMachineValue,
  normalizeWaitingRoomLaunchPlacement,
  waitingRoomLaunchKernelOptions,
  waitingRoomLaunchMachineOptions,
  waitingRoomLaunchPlacement,
  waitingRoomSelectedLaunchKernelRef,
  waitingRoomSelectedLaunchMachineRef,
  managedEnvironmentMachineRef,
} from "./waiting-room-runtime-placement.js"

test("waiting room launch machine options include local and remote labels", () => {
  assert.deepEqual(waitingRoomLaunchMachineOptions(remote()).map((option) => ({
    id: option.id,
    label: option.label,
  })), [
    { id: "local", label: "local" },
    { id: "machine-1", label: "registry-one" },
    { id: "machine-2", label: "display two" },
    { id: "machine-without-kernel", label: "machine-without-kernel" },
  ])
})

test("waiting room launch kernel options scope kernels by selected machine", () => {
  assert.deepEqual(waitingRoomLaunchKernelOptions(remote(), "local").map((option) => option.id), ["local"])
  assert.deepEqual(waitingRoomLaunchKernelOptions(remote(), "machine-1").map((option) => ({
    id: option.id,
    label: option.label,
    machineId: option.machineId,
  })), [
    { id: "kernel-1a", label: "relay-one-a", machineId: "machine-1" },
    { id: "kernel-1b", label: "kernel-one-b", machineId: "machine-1" },
  ])
})

test("waiting room launch placement normalizes stale selections", () => {
  assert.equal(waitingRoomSelectedLaunchMachineRef({ selectedMachineRef: "missing" }, remote()), "local")
  assert.equal(waitingRoomSelectedLaunchKernelRef({
    selectedMachineRef: "machine-1",
    selectedKernelRef: "missing",
  }, remote()), "kernel-1a")
  assert.equal(waitingRoomSelectedLaunchMachineRef({
    selectedMachineRef: "managed:new",
  }), "managed:new")
  assert.equal(waitingRoomSelectedLaunchMachineRef({
    selectedMachineRef: managedEnvironmentMachineRef("environment-stale"),
  }), managedEnvironmentMachineRef("environment-stale"))
  assert.deepEqual(normalizeWaitingRoomLaunchPlacement({
    selectedMachineRef: "machine-2",
    selectedKernelRef: "missing",
  }, remote()), {
    selectedMachineRef: "machine-2",
    selectedKernelRef: "kernel-2a",
  })
})

test("waiting room launch placement resolves launch refs", () => {
  assert.deepEqual(waitingRoomLaunchPlacement({
    selectedMachineRef: "machine-1",
    selectedKernelRef: "kernel-1b",
  }, remote()), {
    machineRef: "machine-1",
    kernelRef: "kernel-1b",
    workerKernelRef: null,
    managedEnvironmentId: null,
    newManagedEnvironment: false,
  })
})

test("waiting room managed machines replace duplicate runtime machines and bind the exact kernel", () => {
  const managedRemote = {
    ...remote(),
    managedEnvironments: [{
      environmentId: "environment-1",
      name: "Managed build",
      desiredState: "running" as const,
      observedState: "ready",
      desiredRevision: 2,
      observedRevision: 2,
      runtimeMachineId: "machine-1",
      runtimeKernelId: "kernel-1b",
    }],
  }
  assert.deepEqual(waitingRoomLaunchMachineOptions(managedRemote).map((option) => option.id), [
    "local",
    "machine-2",
    "machine-without-kernel",
    managedEnvironmentMachineRef("environment-1"),
    "managed:new",
  ])
  assert.deepEqual(waitingRoomLaunchKernelOptions(
    managedRemote,
    managedEnvironmentMachineRef("environment-1"),
  ).map((option) => option.id), ["kernel-1b"])
  assert.deepEqual(waitingRoomLaunchPlacement({
    selectedMachineRef: managedEnvironmentMachineRef("environment-1"),
    selectedKernelRef: "kernel-1b",
  }, managedRemote), {
    machineRef: managedEnvironmentMachineRef("environment-1"),
    kernelRef: "kernel-1b",
    workerKernelRef: null,
    managedEnvironmentId: "environment-1",
    newManagedEnvironment: false,
  })
  assert.deepEqual(cycleWaitingRoomLaunchKernel({
    selectedMachineRef: managedEnvironmentMachineRef("environment-1"),
    selectedKernelRef: "kernel-1b",
  }, managedRemote, 1), {
    selectedMachineRef: managedEnvironmentMachineRef("environment-1"),
    selectedKernelRef: "kernel-1b",
  })
})

test("waiting room launch value labels reflect normalized selections", () => {
  assert.equal(formatWaitingRoomLaunchMachineValue({ selectedMachineRef: "machine-1" }, remote()), "registry-one")
  assert.equal(formatWaitingRoomLaunchKernelValue({
    selectedMachineRef: "machine-1",
    selectedKernelRef: "kernel-1b",
  }, remote()), "kernel-one-b")
  assert.equal(formatWaitingRoomLaunchKernelValue({ selectedMachineRef: "machine-without-kernel" }, remote()), "none available")
})

test("waiting room launch cycling updates machine and kernel selections", () => {
  const state = { selectedMachineRef: "local", selectedKernelRef: "local", kept: true }
  assert.deepEqual(cycleWaitingRoomLaunchMachine(state, remote(), 1), {
    selectedMachineRef: "machine-1",
    selectedKernelRef: "kernel-1a",
    kept: true,
  })
  assert.deepEqual(cycleWaitingRoomLaunchKernel({
    selectedMachineRef: "machine-1",
    selectedKernelRef: "kernel-1a",
    kept: true,
  }, remote(), 1), {
    selectedMachineRef: "machine-1",
    selectedKernelRef: "kernel-1b",
    kept: true,
  })
  assert.deepEqual(cycleWaitingRoomLaunchKernel({
    selectedMachineRef: "machine-without-kernel",
    selectedKernelRef: "",
    kept: true,
  }, remote(), 1), {
    selectedMachineRef: "machine-without-kernel",
    selectedKernelRef: "",
    kept: true,
  })
})

function remote() {
  return {
    machines: [
      {
        machine_id: "machine-1",
        registry_alias: "registry-one",
        machine_alias: "machine one",
        kernel_count: 2,
      },
      {
        machine_id: "machine-2",
        display_name: "display two",
        kernel_count: 1,
      },
      {
        machine_id: "machine-without-kernel",
        kernel_count: 0,
      },
    ],
    kernels: [
      {
        kernel_id: "kernel-1a",
        machine_id: "machine-1",
        relay_alias: "relay-one-a",
      },
      {
        kernel_id: "kernel-1b",
        machine_id: "machine-1",
        kernel_alias: "kernel-one-b",
      },
      {
        kernel_id: "kernel-2a",
        machine_id: "machine-2",
      },
    ],
  }
}
