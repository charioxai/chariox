import assert from "node:assert/strict"
import test from "node:test"

import {
  cycleRoomListLaunchKernel,
  cycleRoomListLaunchMachine,
  formatRoomListLaunchKernelValue,
  formatRoomListLaunchMachineValue,
  normalizeRoomListLaunchPlacement,
  roomListLaunchKernelOptions,
  roomListLaunchMachineOptions,
  roomListLaunchPlacement,
  roomListSelectedLaunchKernelRef,
  roomListSelectedLaunchMachineRef,
} from "./room-list-runtime-placement.js"

test("Room list launch machine options include local and remote labels", () => {
  assert.deepEqual(roomListLaunchMachineOptions(remote()).map((option) => ({
    id: option.id,
    label: option.label,
  })), [
    { id: "local", label: "local" },
    { id: "machine-1", label: "machine one" },
    { id: "machine-2", label: "display two" },
    { id: "machine-without-kernel", label: "machine-without-kernel" },
  ])
})

test("Room list launch kernel options scope kernels by selected machine", () => {
  assert.deepEqual(roomListLaunchKernelOptions(remote(), "local").map((option) => option.id), ["local"])
  assert.deepEqual(roomListLaunchKernelOptions(remote(), "machine-1").map((option) => ({
    id: option.id,
    label: option.label,
    machineId: option.machineId,
  })), [
    { id: "kernel-1a", label: "relay-one-a", machineId: "machine-1" },
    { id: "kernel-1b", label: "kernel-one-b", machineId: "machine-1" },
  ])
})

test("Room list launch placement resolves launch refs", () => {
  assert.deepEqual(roomListLaunchPlacement({
    selectedMachineRef: "machine-1",
    selectedKernelRef: "kernel-1b",
  }, remote()), {
    machineRef: "machine-1",
    kernelRef: "kernel-1b",
    workerKernelRef: null,
    newMachineAllocation: false,
  })
})

test("Room list launch value labels reflect normalized selections", () => {
  assert.equal(formatRoomListLaunchMachineValue({ selectedMachineRef: "machine-1" }, remote()), "machine one")
  assert.equal(formatRoomListLaunchKernelValue({
    selectedMachineRef: "machine-1",
    selectedKernelRef: "kernel-1b",
  }, remote()), "kernel-one-b")
  assert.equal(formatRoomListLaunchKernelValue({ selectedMachineRef: "machine-without-kernel" }, remote()), "none available")
})

test("Room list launch cycling updates machine and kernel selections", () => {
  const state = { selectedMachineRef: "local", selectedKernelRef: "local", kept: true }
  assert.deepEqual(cycleRoomListLaunchMachine(state, remote(), 1), {
    selectedMachineRef: "machine-1",
    selectedKernelRef: "kernel-1a",
    kept: true,
  })
  assert.deepEqual(cycleRoomListLaunchKernel({
    selectedMachineRef: "machine-1",
    selectedKernelRef: "kernel-1a",
    kept: true,
  }, remote(), 1), {
    selectedMachineRef: "machine-1",
    selectedKernelRef: "kernel-1b",
    kept: true,
  })
  assert.deepEqual(cycleRoomListLaunchKernel({
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
