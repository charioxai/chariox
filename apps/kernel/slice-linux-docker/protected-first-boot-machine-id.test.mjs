import assert from "node:assert/strict"
import test from "node:test"
import { prepareIdentityArguments } from "./protected-first-boot.mjs"

const base = { container: "chariox-slice-fixture", containerUid: 1001, port: 43119 }

test("the first-use identity barrier tells the protected slice the machine id its owner passed", () => {
  const args = prepareIdentityArguments({ ...base, machineId: "machine-2397eacb2b313ac674790fd376bc0ea3" })
  const at = args.indexOf("CHARIOX_MACHINE_ID=machine-2397eacb2b313ac674790fd376bc0ea3")
  assert.ok(at > 0 && args[at - 1] === "-e", "the machine id is passed as an environment variable of the exec")
  assert.ok(at < args.indexOf("chariox-slice-fixture"), "environment flags come before the container name")
  assert.deepEqual(args.slice(-2), ["--prepare-protected-slice-identity", "43119"])
})

test("without an owner machine id the barrier passes none", () => {
  assert.ok(!prepareIdentityArguments(base).some(arg => arg.startsWith("CHARIOX_MACHINE_ID=")))
})

test("a machine id that could be mistaken for a flag or a path is refused", () => {
  for (const machineId of ["--privileged", "machine/../x", "machine id", "x".repeat(201)]) {
    assert.throws(() => prepareIdentityArguments({ ...base, machineId }), /Protected identity retention is required/)
  }
})
